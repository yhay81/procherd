use std::{
    fs, io,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    sync::mpsc::{Receiver, TryRecvError},
    thread,
    time::{Duration, Instant},
};

use process_wrap::std::{StdChildWrapper, StdCommandWrap};

#[cfg(windows)]
use process_wrap::std::JobObject;
#[cfg(unix)]
use process_wrap::std::{ProcessGroup, ProcessSession};

use crate::{
    error::AppError,
    leases::LeaseAllocation,
    logs::{LogChunk, LogWriter, capture_stream, channel},
    model::{
        ExitInfo, ExitReason, FailureInfo, RUN_SCHEMA_VERSION, RunState, RunStatus, StopRequest,
        SupervisorSpec,
    },
    readiness::ReadinessTracker,
    store::{atomic_write_json, now_ms, open_supervisor_lock, read_json},
};

const POLL_INTERVAL: Duration = Duration::from_millis(25);
const LOG_DRAIN_LIMIT: Duration = Duration::from_millis(500);

pub fn launch(run_dir: &Path) -> Result<(), AppError> {
    let executable = std::env::current_exe().map_err(|error| {
        AppError::operational(
            "supervisor_launch",
            format!("cannot locate executable: {error}"),
        )
    })?;
    #[cfg(unix)]
    {
        let mut command = StdCommandWrap::with_new(&executable, |command| {
            command
                .arg("__supervise")
                .arg("--run-dir")
                .arg(run_dir)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
        });
        command.wrap(ProcessSession);
        let child = command.spawn().map_err(|error| {
            AppError::operational(
                "supervisor_launch",
                format!("cannot start detached supervisor: {error}"),
            )
        })?;
        drop(child);
    }
    #[cfg(windows)]
    {
        crate::windows_detach::spawn_supervisor(&executable, run_dir).map_err(|error| {
            AppError::operational(
                "supervisor_launch",
                format!("cannot start detached supervisor: {error}"),
            )
        })?;
    }
    Ok(())
}

pub fn supervise(run_dir: PathBuf) -> Result<(), AppError> {
    let _lock = open_supervisor_lock(&run_dir)?;
    let result = supervise_locked(&run_dir);
    if let Err(error) = &result {
        mark_supervisor_failed(&run_dir, error);
    }
    result
}

fn supervise_locked(run_dir: &Path) -> Result<(), AppError> {
    let spec_path = run_dir.join("spec.json");
    let spec: SupervisorSpec = read_json(&spec_path)?;
    if spec.schema_version != "procherd.supervisor-spec.v1" {
        return Err(AppError::integrity(format!(
            "unsupported supervisor spec {}",
            spec.schema_version
        )));
    }
    let mut state: RunState = read_json(&run_dir.join("state.json"))?;
    if state.schema_version != RUN_SCHEMA_VERSION
        || state.run_id != spec.run_id
        || state.command.program != spec.command.program
        || state.command.args != spec.command.args
        || state.readiness != spec.readiness
        || state.limits.max_runtime_ms != spec.max_runtime_ms
        || state.limits.runtime_grace_ms != spec.runtime_grace_ms
    {
        return Err(AppError::integrity(
            "supervisor spec does not match run state",
        ));
    }
    fs::remove_file(&spec_path)?;

    state.status = RunStatus::Starting;
    state.updated_at_ms = now_ms();
    state.process.supervisor_pid = Some(std::process::id());
    persist_state(run_dir, &state)?;

    let root = run_dir
        .parent()
        .ok_or_else(|| AppError::integrity("run directory has no state-root parent"))?;
    let mut leases = LeaseAllocation::acquire(root, run_dir, &mut state, &spec.lease_requests)?;
    persist_state(run_dir, &state)?;

    let mut command = StdCommandWrap::with_new(&state.command.program, |command| {
        command
            .args(&state.command.args)
            .current_dir(&state.command.working_directory)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
    });
    command
        .command_mut()
        .envs(leases.environment().iter().cloned());
    #[cfg(unix)]
    command.wrap(ProcessGroup::leader());
    #[cfg(windows)]
    command.wrap(JobObject);

    leases.handoff(&mut state);
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            state.status = RunStatus::Failed;
            state.updated_at_ms = now_ms();
            state.exit = Some(ExitInfo {
                code: None,
                signal: None,
                reason: ExitReason::SpawnFailed,
                finished_at_ms: now_ms(),
            });
            state.failure = Some(FailureInfo {
                kind: "process_spawn".to_owned(),
                message: error.to_string(),
            });
            leases.release(&mut state)?;
            persist_state(run_dir, &state)?;
            return Err(AppError::operational(
                "process_spawn",
                format!("cannot start {}: {error}", spec.command.program),
            ));
        }
    };
    let mut child = OwnedChild::new(child);
    let spawned_at_ms = now_ms();
    leases.record_spawn(&mut state, spawned_at_ms);
    state.limits.runtime_deadline_at_ms = spec
        .max_runtime_ms
        .map(|runtime| spawned_at_ms.saturating_add(runtime));

    state.status = RunStatus::Running;
    state.updated_at_ms = now_ms();
    state.process.process_pid = Some(child.as_mut().id());
    persist_state(run_dir, &state)?;

    let (sender, receiver) = channel();
    if let Some(stdout) = child.as_mut().stdout().take() {
        capture_stream(crate::model::LogStream::Stdout, stdout, sender.clone());
    }
    if let Some(stderr) = child.as_mut().stderr().take() {
        capture_stream(crate::model::LogStream::Stderr, stderr, sender.clone());
    }
    drop(sender);

    let mut logs = LogWriter::create(run_dir, spec.max_log_bytes)?;
    let mut readiness = ReadinessTracker::new();
    let mut stopping: Option<Stopping> = None;
    let mut last_state_write = Instant::now();

    loop {
        let mut readiness_changed =
            drain_available(&receiver, &mut logs, &mut readiness, &mut state)?;
        readiness_changed |= readiness.poll(&mut state.readiness);
        if readiness_changed {
            state.updated_at_ms = now_ms();
            persist_state(run_dir, &state)?;
            last_state_write = Instant::now();
        }

        if stopping.is_none() {
            if let Some(request) = read_stop_request(run_dir, &spec)? {
                state.status = RunStatus::Stopping;
                state.updated_at_ms = now_ms();
                state.cleanup.requested_at_ms = Some(request.requested_at_ms);
                state.cleanup.grace_ms = Some(request.grace_ms);
                stopping = Some(request_stop(
                    child.as_mut(),
                    request.grace_ms,
                    ExitReason::StopRequested,
                    &mut state,
                )?);
                persist_state(run_dir, &state)?;
            } else if state
                .limits
                .runtime_deadline_at_ms
                .is_some_and(|deadline| now_ms() >= deadline)
            {
                let triggered_at_ms = now_ms();
                state.status = RunStatus::Stopping;
                state.updated_at_ms = triggered_at_ms;
                state.limits.runtime_limit_triggered_at_ms = Some(triggered_at_ms);
                state.cleanup.requested_at_ms = Some(triggered_at_ms);
                state.cleanup.grace_ms = Some(spec.runtime_grace_ms);
                stopping = Some(request_stop(
                    child.as_mut(),
                    spec.runtime_grace_ms,
                    ExitReason::RuntimeLimit,
                    &mut state,
                )?);
                persist_state(run_dir, &state)?;
            }
        } else if let Some(stopping_state) = &mut stopping {
            if !stopping_state.force_sent && Instant::now() >= stopping_state.deadline {
                if start_kill_tolerant(child.as_mut())? {
                    state.cleanup.descendant_cleanup_triggered = true;
                }
                stopping_state.force_sent = true;
                state.cleanup.force_used = true;
                state.updated_at_ms = now_ms();
                persist_state(run_dir, &state)?;
            }
        }

        match try_wait_retry(child.as_mut())? {
            Some(exit_status) => {
                if start_kill_tolerant(child.as_mut())? {
                    state.cleanup.descendant_cleanup_triggered = true;
                    if stopping.is_some() {
                        state.cleanup.force_used = true;
                    }
                }
                child.disarm();
                drain_until_closed(
                    &receiver,
                    &mut logs,
                    &mut readiness,
                    &mut state,
                    LOG_DRAIN_LIMIT,
                )?;
                ReadinessTracker::process_exited(&mut state.readiness);
                state.logs = logs.finish()?;
                leases.release(&mut state)?;
                state.updated_at_ms = now_ms();
                state.cleanup.completed_at_ms = Some(state.updated_at_ms);
                state.status = if stopping.is_some() {
                    RunStatus::Stopped
                } else {
                    RunStatus::Exited
                };
                state.exit = Some(exit_info(
                    exit_status,
                    stopping
                        .as_ref()
                        .map_or(ExitReason::ProcessExited, |value| value.reason),
                ));
                persist_state(run_dir, &state)?;
                return Ok(());
            }
            None => {
                if last_state_write.elapsed() >= Duration::from_millis(250) {
                    state.logs = logs.snapshot();
                    state.updated_at_ms = now_ms();
                    persist_state(run_dir, &state)?;
                    last_state_write = Instant::now();
                }
                thread::sleep(POLL_INTERVAL);
            }
        }
    }
}

struct Stopping {
    deadline: Instant,
    force_sent: bool,
    reason: ExitReason,
}

fn request_stop(
    child: &mut dyn StdChildWrapper,
    grace_ms: u64,
    reason: ExitReason,
    state: &mut RunState,
) -> Result<Stopping, AppError> {
    state.cleanup.descendant_cleanup_triggered = true;
    #[cfg(unix)]
    {
        match child.signal(15) {
            Ok(()) => {}
            Err(error) if is_process_gone(&error) => {}
            Err(error) => return Err(error.into()),
        }
        Ok(Stopping {
            deadline: Instant::now() + Duration::from_millis(grace_ms),
            force_sent: false,
            reason,
        })
    }
    #[cfg(windows)]
    {
        let _ = grace_ms;
        start_kill_tolerant(child)?;
        state.cleanup.force_used = true;
        Ok(Stopping {
            deadline: Instant::now(),
            force_sent: true,
            reason,
        })
    }
}

fn start_kill_tolerant(child: &mut dyn StdChildWrapper) -> Result<bool, AppError> {
    match child.start_kill() {
        Ok(()) => Ok(true),
        Err(error) if is_process_gone(&error) => Ok(false),
        Err(error) => Err(error.into()),
    }
}

fn is_process_gone(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::InvalidInput
    ) || error.raw_os_error() == Some(3)
}

fn try_wait_retry(child: &mut dyn StdChildWrapper) -> Result<Option<ExitStatus>, AppError> {
    loop {
        match child.try_wait() {
            Ok(status) => return Ok(status),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.into()),
        }
    }
}

fn read_stop_request(
    run_dir: &Path,
    spec: &SupervisorSpec,
) -> Result<Option<StopRequest>, AppError> {
    let path = run_dir.join("stop.request.json");
    let request: StopRequest = match read_json(&path) {
        Ok(request) => request,
        Err(error) if error.kind == "io" && !path.exists() => return Ok(None),
        Err(error) => return Err(error),
    };
    if request.schema_version != "procherd.stop-request.v1"
        || request.run_id != spec.run_id
        || request.owner_token != spec.owner_token
    {
        return Err(AppError::integrity(
            "stop request failed run identity or owner-token validation",
        ));
    }
    fs::remove_file(&path)?;
    Ok(Some(request))
}

fn drain_available(
    receiver: &Receiver<LogChunk>,
    logs: &mut LogWriter,
    readiness: &mut ReadinessTracker,
    state: &mut RunState,
) -> Result<bool, AppError> {
    let mut readiness_changed = false;
    loop {
        match receiver.try_recv() {
            Ok(chunk) => {
                readiness_changed |= readiness.observe_log(
                    &mut state.readiness,
                    chunk.stream,
                    logs.capturable_bytes(&chunk),
                );
                logs.record(chunk)?;
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => {
                return Ok(readiness_changed);
            }
        }
    }
}

fn drain_until_closed(
    receiver: &Receiver<LogChunk>,
    logs: &mut LogWriter,
    readiness: &mut ReadinessTracker,
    state: &mut RunState,
    limit: Duration,
) -> Result<(), AppError> {
    let deadline = Instant::now() + limit;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            drain_available(receiver, logs, readiness, state)?;
            return Ok(());
        }
        match receiver.recv_timeout(remaining.min(POLL_INTERVAL)) {
            Ok(chunk) => {
                readiness.observe_log(
                    &mut state.readiness,
                    chunk.stream,
                    logs.capturable_bytes(&chunk),
                );
                logs.record(chunk)?;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

fn persist_state(run_dir: &Path, state: &RunState) -> Result<(), AppError> {
    atomic_write_json(&run_dir.join("state.json"), state)
}

fn mark_supervisor_failed(run_dir: &Path, error: &AppError) {
    if let Ok(mut state) = read_json::<RunState>(&run_dir.join("state.json")) {
        if !state.status.is_terminal() {
            state.status = RunStatus::Failed;
            state.updated_at_ms = now_ms();
            state.exit = Some(ExitInfo {
                code: None,
                signal: None,
                reason: ExitReason::SupervisorFailed,
                finished_at_ms: now_ms(),
            });
            state.failure = Some(FailureInfo {
                kind: error.kind.to_owned(),
                message: error.message.clone(),
            });
            state.cleanup.completed_at_ms = Some(state.updated_at_ms);
            for lease in &mut state.leases.ports {
                if lease.acquired_at_ms.is_some() && lease.released_at_ms.is_none() {
                    lease.released_at_ms = Some(state.updated_at_ms);
                }
            }
            for lease in &mut state.leases.temp_directories {
                if lease.acquired_at_ms.is_some() && lease.released_at_ms.is_none() {
                    lease.released_at_ms = Some(state.updated_at_ms);
                }
            }
            let _ = persist_state(run_dir, &state);
        }
    }
}

fn exit_info(status: ExitStatus, reason: ExitReason) -> ExitInfo {
    #[cfg(unix)]
    use std::os::unix::process::ExitStatusExt;

    ExitInfo {
        code: status.code(),
        #[cfg(unix)]
        signal: status.signal(),
        #[cfg(not(unix))]
        signal: None,
        reason,
        finished_at_ms: now_ms(),
    }
}

struct OwnedChild {
    inner: Option<Box<dyn StdChildWrapper>>,
}

impl OwnedChild {
    fn new(child: Box<dyn StdChildWrapper>) -> Self {
        Self { inner: Some(child) }
    }

    fn as_mut(&mut self) -> &mut dyn StdChildWrapper {
        self.inner
            .as_deref_mut()
            .expect("owned child is present until disarmed")
    }

    fn disarm(&mut self) {
        self.inner.take();
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if let Some(child) = &mut self.inner {
            let _ = child.start_kill();
        }
    }
}
