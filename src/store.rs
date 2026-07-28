use std::{
    env,
    fs::{self, File, OpenOptions},
    io::{self, BufReader, BufWriter, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use fs4::{FileExt, TryLockError};
use serde::{Serialize, de::DeserializeOwned};
use ulid::Ulid;

use crate::{
    error::AppError,
    model::{RunState, StopRequest, SupervisorSpec},
};

const DOCUMENT_LIMIT: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    pub fn resolve(explicit: Option<PathBuf>) -> Result<Self, AppError> {
        let root = if let Some(path) = explicit {
            path
        } else if let Some(path) = env::var_os("PROCHERD_STATE_DIR") {
            PathBuf::from(path)
        } else {
            platform_data_dir()?.join("procherd")
        };
        create_private_dir(&root)?;
        let root = fs::canonicalize(&root).map_err(|error| {
            AppError::operational(
                "state_directory",
                format!(
                    "cannot canonicalize state directory {}: {error}",
                    root.display()
                ),
            )
        })?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn create_run(
        &self,
        state: &RunState,
        spec: &SupervisorSpec,
        owner_token: &str,
    ) -> Result<PathBuf, AppError> {
        validate_run_id(&state.run_id)?;
        let run_dir = self.root.join(&state.run_id);
        create_new_private_dir(&run_dir)?;
        create_private_file(&run_dir.join("supervisor.lock"))?;
        write_new_json(&run_dir.join("state.json"), state)?;
        write_new_json(&run_dir.join("spec.json"), spec)?;
        write_new_private_text(&run_dir.join("owner.token"), owner_token)?;
        Ok(run_dir)
    }

    pub fn run_dir(&self, run_id: &str) -> Result<PathBuf, AppError> {
        validate_run_id(run_id)?;
        let run_dir = self.root.join(run_id);
        match fs::symlink_metadata(&run_dir) {
            Ok(metadata) if metadata.file_type().is_symlink() => Err(AppError::integrity(format!(
                "run directory is a symbolic link: {}",
                run_dir.display()
            ))),
            Ok(metadata) if !metadata.is_dir() => Err(AppError::integrity(format!(
                "run path is not a directory: {}",
                run_dir.display()
            ))),
            Ok(_) => Ok(run_dir),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                Err(AppError::not_found(format!("run not found: {run_id}")))
            }
            Err(error) => Err(error.into()),
        }
    }

    pub fn read_state(&self, run_id: &str) -> Result<RunState, AppError> {
        let run_dir = self.run_dir(run_id)?;
        let state: RunState = read_json(&run_dir.join("state.json"))?;
        if state.run_id != run_id {
            return Err(AppError::integrity(format!(
                "state run ID {} does not match directory {run_id}",
                state.run_id
            )));
        }
        Ok(state)
    }

    pub fn read_owner_token(&self, run_id: &str) -> Result<String, AppError> {
        let run_dir = self.run_dir(run_id)?;
        let token = read_bounded_text(&run_dir.join("owner.token"), 256)?;
        let token = token.trim().to_owned();
        if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(AppError::integrity("owner token is malformed"));
        }
        Ok(token)
    }

    pub fn write_stop_request(&self, request: &StopRequest) -> Result<(), AppError> {
        let run_dir = self.run_dir(&request.run_id)?;
        atomic_write_json(&run_dir.join("stop.request.json"), request)
    }

    pub fn supervisor_active(&self, run_id: &str) -> Result<bool, AppError> {
        let run_dir = self.run_dir(run_id)?;
        let path = run_dir.join("supervisor.lock");
        ensure_regular_file(&path)?;
        let lock = OpenOptions::new().read(true).write(true).open(path)?;
        match FileExt::try_lock(&lock) {
            Ok(()) => {
                FileExt::unlock(&lock)?;
                Ok(false)
            }
            Err(TryLockError::WouldBlock) => Ok(true),
            Err(TryLockError::Error(error)) => Err(error.into()),
        }
    }

    pub fn list_run_ids(&self) -> Result<Vec<String>, AppError> {
        let mut ids = Vec::new();
        for entry in fs::read_dir(&self.root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if validate_run_id(&name).is_ok() {
                ids.push(name);
            }
        }
        ids.sort_unstable();
        ids.reverse();
        Ok(ids)
    }

    pub fn remove_terminal_run(&self, run_id: &str) -> Result<(), AppError> {
        let run_dir = self.run_dir(run_id)?;
        if self.supervisor_active(run_id)? {
            return Err(AppError::operational(
                "gc_live_run",
                format!("refusing to remove active run {run_id}"),
            ));
        }
        let state = self.read_state(run_id)?;
        if !state.status.is_terminal() {
            return Err(AppError::operational(
                "gc_live_run",
                format!("refusing to remove non-terminal run {run_id}"),
            ));
        }
        fs::remove_dir_all(run_dir)?;
        Ok(())
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

pub fn validate_run_id(run_id: &str) -> Result<(), AppError> {
    let suffix = run_id
        .strip_prefix("run_")
        .ok_or_else(|| AppError::usage("run ID must start with run_"))?;
    let parsed = suffix.parse::<Ulid>().map_err(|_| {
        AppError::usage("run ID must contain a canonical 26-character uppercase ULID")
    })?;
    if suffix.len() != 26 || parsed.to_string() != suffix {
        return Err(AppError::usage(
            "run ID must contain a canonical 26-character uppercase ULID",
        ));
    }
    Ok(())
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, AppError> {
    ensure_regular_file(path)?;
    let file = File::open(path)?;
    let metadata = file.metadata()?;
    if metadata.len() > DOCUMENT_LIMIT {
        return Err(AppError::integrity(format!(
            "document exceeds {} bytes: {}",
            DOCUMENT_LIMIT,
            path.display()
        )));
    }
    serde_json::from_reader(BufReader::new(file)).map_err(|error| {
        AppError::integrity(format!("invalid JSON document {}: {error}", path.display()))
    })
}

pub fn write_new_json<T: Serialize>(path: &Path, value: &T) -> Result<(), AppError> {
    let file = create_new_private_file(path)?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.write_all(b"\n")?;
    let file = writer
        .into_inner()
        .map_err(|error| AppError::from(error.into_error()))?;
    file.sync_all()?;
    Ok(())
}

pub fn atomic_write_json<T: Serialize>(path: &Path, value: &T) -> Result<(), AppError> {
    #[cfg(unix)]
    let mut options = atomic_write_file::OpenOptions::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(not(unix))]
    let options = atomic_write_file::OpenOptions::new();
    let mut writer = options.open(path)?;
    serde_json::to_writer_pretty(&mut writer, value)?;
    writer.write_all(b"\n")?;
    writer.flush()?;
    writer.commit()?;
    Ok(())
}

pub fn read_bounded_text(path: &Path, limit: u64) -> Result<String, AppError> {
    ensure_regular_file(path)?;
    let file = File::open(path)?;
    if file.metadata()?.len() > limit {
        return Err(AppError::integrity(format!(
            "file exceeds {limit} bytes: {}",
            path.display()
        )));
    }
    let mut value = String::new();
    BufReader::new(file)
        .take(limit + 1)
        .read_to_string(&mut value)?;
    if u64::try_from(value.len()).unwrap_or(u64::MAX) > limit {
        return Err(AppError::integrity(format!(
            "file exceeds {limit} bytes: {}",
            path.display()
        )));
    }
    Ok(value)
}

pub fn open_supervisor_lock(run_dir: &Path) -> Result<File, AppError> {
    let path = run_dir.join("supervisor.lock");
    ensure_regular_file(&path)?;
    let file = OpenOptions::new().read(true).write(true).open(path)?;
    FileExt::lock(&file)?;
    Ok(file)
}

fn platform_data_dir() -> Result<PathBuf, AppError> {
    #[cfg(target_os = "windows")]
    {
        env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| AppError::operational("state_directory", "LOCALAPPDATA is not defined"))
    }
    #[cfg(target_os = "macos")]
    {
        env::var_os("HOME")
            .map(|home| PathBuf::from(home).join("Library/Application Support"))
            .ok_or_else(|| AppError::operational("state_directory", "HOME is not defined"))
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Some(path) = env::var_os("XDG_DATA_HOME") {
            return Ok(PathBuf::from(path));
        }
        env::var_os("HOME")
            .map(|home| PathBuf::from(home).join(".local/share"))
            .ok_or_else(|| {
                AppError::operational(
                    "state_directory",
                    "neither XDG_DATA_HOME nor HOME is defined",
                )
            })
    }
}

fn create_private_dir(path: &Path) -> Result<(), AppError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder.create(path)?;
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(path)?;
    }
    Ok(())
}

fn create_new_private_dir(path: &Path) -> Result<(), AppError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(path)?;
    }
    #[cfg(not(unix))]
    {
        fs::create_dir(path)?;
    }
    Ok(())
}

fn create_private_file(path: &Path) -> Result<File, AppError> {
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

pub fn create_new_private_file(path: &Path) -> Result<File, AppError> {
    let mut options = OpenOptions::new();
    options.create_new(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}

fn write_new_private_text(path: &Path, value: &str) -> Result<(), AppError> {
    let mut file = create_new_private_file(path)?;
    file.write_all(value.as_bytes())?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

pub fn ensure_regular_file(path: &Path) -> Result<(), AppError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(AppError::integrity(format!(
            "refusing symbolic-link file: {}",
            path.display()
        )));
    }
    if !metadata.is_file() {
        return Err(AppError::integrity(format!(
            "expected a regular file: {}",
            path.display()
        )));
    }
    Ok(())
}
