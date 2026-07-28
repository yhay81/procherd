use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs::{self, File, OpenOptions},
    net::TcpListener,
    path::{Path, PathBuf},
};

use fs4::{FileExt, TryLockError};
use serde::{Deserialize, Serialize};

use crate::{
    error::AppError,
    model::{LeaseRequests, ReadinessCondition, RunState},
    store::{atomic_write_json, now_ms, read_json},
};

const REGISTRY_SCHEMA: &str = "procherd.port-leases.v1";
const MAX_LEASES_PER_KIND: usize = 16;
const MAX_ALLOCATION_ATTEMPTS: usize = 128;

#[derive(Debug, Default, Deserialize, Serialize)]
struct PortRegistry {
    schema_version: String,
    entries: Vec<PortRegistryEntry>,
}

#[derive(Debug, Deserialize, Serialize)]
struct PortRegistryEntry {
    run_id: String,
    name: String,
    port: u16,
    acquired_at_ms: u64,
}

pub fn validate_requests(
    port_names: Vec<String>,
    temp_directory_names: Vec<String>,
) -> Result<LeaseRequests, AppError> {
    if port_names.len() > MAX_LEASES_PER_KIND || temp_directory_names.len() > MAX_LEASES_PER_KIND {
        return Err(AppError::usage(format!(
            "at most {MAX_LEASES_PER_KIND} port and {MAX_LEASES_PER_KIND} temporary-directory leases may be requested"
        )));
    }
    validate_names(&port_names)?;
    validate_names(&temp_directory_names)?;
    Ok(LeaseRequests {
        port_names,
        temp_directory_names,
    })
}

pub struct LeaseAllocation {
    root: PathBuf,
    run_id: String,
    ports: Vec<ReservedPort>,
    environment: Vec<(String, OsString)>,
    registry_released: bool,
}

struct ReservedPort {
    name: String,
    port: u16,
    listener: Option<TcpListener>,
}

impl LeaseAllocation {
    pub fn acquire(
        root: &Path,
        run_dir: &Path,
        state: &mut RunState,
        requests: &LeaseRequests,
    ) -> Result<Self, AppError> {
        let acquired_at_ms = now_ms();
        let temp_paths = create_temp_directories(run_dir, &requests.temp_directory_names)?;
        let ports = reserve_ports(root, &state.run_id, &requests.port_names)?;
        let mut allocation = Self {
            root: root.to_path_buf(),
            run_id: state.run_id.clone(),
            ports,
            environment: Vec::new(),
            registry_released: false,
        };
        let result = allocation.populate_state(state, acquired_at_ms, &temp_paths);
        if let Err(error) = result {
            let _ = allocation.release_registry();
            return Err(error);
        }
        Ok(allocation)
    }

    pub fn environment(&self) -> &[(String, OsString)] {
        &self.environment
    }

    pub fn handoff(&mut self, state: &mut RunState) -> u64 {
        let handoff_at_ms = now_ms();
        for reserved in &mut self.ports {
            reserved.listener.take();
            if let Some(port) = state
                .leases
                .ports
                .iter_mut()
                .find(|lease| lease.name == reserved.name)
            {
                port.handoff_at_ms = Some(handoff_at_ms);
            }
        }
        handoff_at_ms
    }

    pub fn record_spawn(&self, state: &mut RunState, spawned_at_ms: u64) {
        for lease in &mut state.leases.ports {
            if let Some(handoff) = lease.handoff_at_ms {
                lease.handoff_gap_ms = Some(spawned_at_ms.saturating_sub(handoff));
            }
        }
    }

    pub fn release(&mut self, state: &mut RunState) -> Result<(), AppError> {
        self.release_registry()?;
        let released_at_ms = now_ms();
        for lease in &mut state.leases.ports {
            lease.released_at_ms = Some(released_at_ms);
        }
        for lease in &mut state.leases.temp_directories {
            lease.released_at_ms = Some(released_at_ms);
        }
        Ok(())
    }

    fn populate_state(
        &mut self,
        state: &mut RunState,
        acquired_at_ms: u64,
        temp_paths: &BTreeMap<String, PathBuf>,
    ) -> Result<(), AppError> {
        let port_values = self
            .ports
            .iter()
            .map(|reserved| (reserved.name.clone(), reserved.port.to_string()))
            .collect::<BTreeMap<_, _>>();
        let temp_values = temp_paths
            .iter()
            .map(|(name, path)| {
                path.to_str()
                    .map(|value| (name.clone(), value.to_owned()))
                    .ok_or_else(|| {
                        AppError::operational(
                            "lease_path",
                            format!("temporary lease path is not UTF-8: {}", path.display()),
                        )
                    })
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?;

        for lease in &mut state.leases.ports {
            let reserved = self
                .ports
                .iter()
                .find(|reserved| reserved.name == lease.name)
                .ok_or_else(|| AppError::integrity("port lease state does not match requests"))?;
            lease.address = Some(format!("127.0.0.1:{}", reserved.port));
            lease.port = Some(reserved.port);
            lease.acquired_at_ms = Some(acquired_at_ms);
            let environment_name = environment_name("PORT", &lease.name);
            self.environment.push((
                environment_name.clone(),
                OsString::from(reserved.port.to_string()),
            ));
            state
                .command
                .environment
                .injected_names
                .push(environment_name);
        }
        for lease in &mut state.leases.temp_directories {
            let path = temp_paths.get(&lease.name).ok_or_else(|| {
                AppError::integrity("temporary-directory lease state does not match requests")
            })?;
            lease.path = Some(path.clone());
            lease.acquired_at_ms = Some(acquired_at_ms);
            let environment_name = environment_name("TEMP", &lease.name);
            self.environment
                .push((environment_name.clone(), path.as_os_str().to_owned()));
            state
                .command
                .environment
                .injected_names
                .push(environment_name);
        }
        state.command.environment.injected_names.sort_unstable();
        state.command.environment.injected_names.dedup();
        for argument in &mut state.command.args {
            *argument = replace_placeholders(argument, &port_values, &temp_values)?;
        }
        for check in &mut state.readiness.checks {
            if let ReadinessCondition::PortLease { name } = &check.condition {
                let port = port_values.get(name).ok_or_else(|| {
                    AppError::integrity(format!("readiness references unknown port lease {name}"))
                })?;
                check.condition = ReadinessCondition::Tcp {
                    address: format!("127.0.0.1:{port}"),
                };
            }
        }
        Ok(())
    }

    fn release_registry(&mut self) -> Result<(), AppError> {
        if self.registry_released {
            return Ok(());
        }
        update_registry(&self.root, |registry| {
            registry.entries.retain(|entry| entry.run_id != self.run_id);
            Ok(())
        })?;
        self.registry_released = true;
        Ok(())
    }
}

impl Drop for LeaseAllocation {
    fn drop(&mut self) {
        let _ = self.release_registry();
    }
}

fn reserve_ports(
    root: &Path,
    run_id: &str,
    names: &[String],
) -> Result<Vec<ReservedPort>, AppError> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let mut reserved = Vec::with_capacity(names.len());
    update_registry(root, |registry| {
        prune_stale(root, registry);
        let mut unavailable = registry
            .entries
            .iter()
            .map(|entry| entry.port)
            .collect::<BTreeSet<_>>();
        for name in names {
            let mut allocated = None;
            for _ in 0..MAX_ALLOCATION_ATTEMPTS {
                let listener = TcpListener::bind(("127.0.0.1", 0))?;
                let port = listener.local_addr()?.port();
                if unavailable.insert(port) {
                    allocated = Some((port, listener));
                    break;
                }
            }
            let (port, listener) = allocated.ok_or_else(|| {
                AppError::operational(
                    "port_lease",
                    "could not find a free port outside the ProcHerd lease registry",
                )
            })?;
            registry.entries.push(PortRegistryEntry {
                run_id: run_id.to_owned(),
                name: name.clone(),
                port,
                acquired_at_ms: now_ms(),
            });
            reserved.push(ReservedPort {
                name: name.clone(),
                port,
                listener: Some(listener),
            });
        }
        Ok(())
    })?;
    Ok(reserved)
}

fn update_registry(
    root: &Path,
    update: impl FnOnce(&mut PortRegistry) -> Result<(), AppError>,
) -> Result<(), AppError> {
    let lock = open_registry_lock(root)?;
    FileExt::lock(&lock)?;
    let path = root.join("port-leases.json");
    let mut registry = if path.exists() {
        read_json(&path)?
    } else {
        PortRegistry {
            schema_version: REGISTRY_SCHEMA.to_owned(),
            entries: Vec::new(),
        }
    };
    if registry.schema_version != REGISTRY_SCHEMA {
        FileExt::unlock(&lock)?;
        return Err(AppError::integrity(format!(
            "unsupported port lease registry schema {}",
            registry.schema_version
        )));
    }
    let result = update(&mut registry).and_then(|()| atomic_write_json(&path, &registry));
    FileExt::unlock(&lock)?;
    result
}

fn prune_stale(root: &Path, registry: &mut PortRegistry) {
    registry
        .entries
        .retain(|entry| run_supervisor_active(root, &entry.run_id));
}

fn run_supervisor_active(root: &Path, run_id: &str) -> bool {
    let path = root.join(run_id).join("supervisor.lock");
    let Ok(metadata) = fs::symlink_metadata(&path) else {
        return false;
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return false;
    }
    let Ok(lock) = OpenOptions::new().read(true).write(true).open(path) else {
        return false;
    };
    match FileExt::try_lock(&lock) {
        Ok(()) => {
            let _ = FileExt::unlock(&lock);
            false
        }
        Err(TryLockError::WouldBlock) => true,
        Err(TryLockError::Error(_)) => false,
    }
}

fn open_registry_lock(root: &Path) -> Result<File, AppError> {
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(root.join("port-leases.lock"))?)
}

fn create_temp_directories(
    run_dir: &Path,
    names: &[String],
) -> Result<BTreeMap<String, PathBuf>, AppError> {
    if names.is_empty() {
        return Ok(BTreeMap::new());
    }
    let root = run_dir.join("resources").join("temp");
    create_private_dir(&root)?;
    let mut paths = BTreeMap::new();
    for name in names {
        let path = root.join(name);
        create_new_private_dir(&path)?;
        paths.insert(name.clone(), path);
    }
    Ok(paths)
}

fn validate_names(names: &[String]) -> Result<(), AppError> {
    let mut unique = BTreeSet::new();
    for name in names {
        let valid = (1..=32).contains(&name.len())
            && name
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_lowercase())
            && name.bytes().all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
            });
        if !valid {
            return Err(AppError::usage(format!(
                "invalid lease name {name:?}; use 1-32 lowercase ASCII letters, digits, _ or -, starting with a letter"
            )));
        }
        if !unique.insert(name) {
            return Err(AppError::usage(format!("duplicate lease name {name:?}")));
        }
    }
    Ok(())
}

fn replace_placeholders(
    value: &str,
    ports: &BTreeMap<String, String>,
    temp_directories: &BTreeMap<String, String>,
) -> Result<String, AppError> {
    let mut result = value.to_owned();
    let contains_temp_placeholder = temp_directories
        .keys()
        .any(|name| value.contains(&format!("{{temp:{name}}}")));
    for (name, port) in ports {
        result = result.replace(&format!("{{port:{name}}}"), port);
    }
    for (name, path) in temp_directories {
        result = result.replace(&format!("{{temp:{name}}}"), path);
    }
    #[cfg(windows)]
    if contains_temp_placeholder {
        result = normalize_verbatim_path_separators(result);
    }
    #[cfg(not(windows))]
    let _ = contains_temp_placeholder;
    if result.contains("{port:") || result.contains("{temp:") {
        return Err(AppError::usage(format!(
            "argument contains an unknown or unterminated lease placeholder: {value}"
        )));
    }
    Ok(result)
}

#[cfg(windows)]
fn normalize_verbatim_path_separators(mut value: String) -> String {
    if let Some(path_start) = value.find(r"\\?\") {
        let suffix = value[path_start..].replace('/', r"\");
        value.truncate(path_start);
        value.push_str(&suffix);
    }
    value
}

fn environment_name(kind: &str, name: &str) -> String {
    let normalized = name.replace('-', "_").to_ascii_uppercase();
    format!("PROCHERD_{kind}_{normalized}")
}

#[cfg(unix)]
fn create_private_dir(path: &Path) -> Result<(), AppError> {
    use std::os::unix::fs::DirBuilderExt;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700);
    builder.create(path)?;
    Ok(())
}

#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> Result<(), AppError> {
    fs::create_dir_all(path)?;
    Ok(())
}

#[cfg(unix)]
fn create_new_private_dir(path: &Path) -> Result<(), AppError> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(path)?;
    Ok(())
}

#[cfg(not(unix))]
fn create_new_private_dir(path: &Path) -> Result<(), AppError> {
    fs::create_dir(path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{replace_placeholders, validate_requests};
    use std::collections::BTreeMap;

    #[test]
    fn lease_names_and_placeholders_are_strict() {
        assert!(validate_requests(vec!["web".to_owned()], vec!["build-dir".to_owned()]).is_ok());
        assert!(validate_requests(vec!["WEB".to_owned()], vec![]).is_err());
        let ports = BTreeMap::from([("web".to_owned(), "43123".to_owned())]);
        let temp = BTreeMap::from([("build".to_owned(), "/tmp/build".to_owned())]);
        assert_eq!(
            replace_placeholders("127.0.0.1:{port:web}:{temp:build}", &ports, &temp,).unwrap(),
            "127.0.0.1:43123:/tmp/build"
        );
        assert!(replace_placeholders("{port:missing}", &ports, &temp).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn temp_placeholder_suffix_uses_verbatim_windows_path_separators() {
        let temp = BTreeMap::from([(
            "build".to_owned(),
            r"\\?\C:\state\resources\temp\build".to_owned(),
        )]);
        assert_eq!(
            replace_placeholders("{temp:build}/nested/artifact.log", &BTreeMap::new(), &temp)
                .unwrap(),
            r"\\?\C:\state\resources\temp\build\nested\artifact.log"
        );
    }
}
