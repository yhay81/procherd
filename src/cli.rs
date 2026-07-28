use std::{
    env,
    ffi::OsString,
    fs::OpenOptions,
    io::{self, Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::{Command, ExitCode},
    thread,
    time::{Duration, Instant},
};

use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use clap_complete::{Generator, Shell, generate};
use schemars::schema_for;
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use ulid::Ulid;

use crate::{
    error::{AppError, ErrorDocument},
    leases::validate_requests,
    logs::{decode_record, read_logs},
    model::{
        CleanupInfo, CommandSpec, EnvironmentMode, EnvironmentPolicy, GcEntry, GcResult,
        LeaseState, LeasesResult, ListResult, LogStream, ObservedStatus, ProcessInfo,
        RUN_SCHEMA_VERSION, ReadinessState, ReadinessStatus, RunLimits, RunState, RunStatus,
        RunView, StartResult, StatusResult, StopRequest, StopResult, SupervisorSpec, TreeControl,
        WaitCondition, WaitResult,
    },
    readiness::build_conditions,
    store::{Store, atomic_write_json, now_ms},
    supervisor,
};

const DEFAULT_LOG_BYTES: u64 = 16 * 1024 * 1024;
const DEFAULT_STARTUP_TIMEOUT: &str = "5s";
const DEFAULT_WAIT_TIMEOUT: &str = "30s";
const DEFAULT_STOP_GRACE: &str = "5s";

#[derive(Debug, Parser)]
#[command(
    name = "procherd",
    version,
    about = "Durable local process control for software agents",
    long_about = "Start bounded local processes under a durable per-run supervisor, then reacquire them by stable run ID to inspect status, wait, read cursor-based logs, or stop the process tree."
)]
struct Cli {
    #[arg(
        long,
        global = true,
        env = "PROCHERD_STATE_DIR",
        help = "State root; defaults to the platform user data directory"
    )]
    state_dir: Option<PathBuf>,

    #[arg(long, global = true, default_value = "human")]
    format: OutputFormat,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum OutputFormat {
    Human,
    Json,
    Ndjson,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Emit the brief contract or a full JSON Schema document.
    Schema {
        #[arg(long, value_enum, default_value = "brief")]
        document: SchemaDocument,
    },
    /// Start a command under a detached per-run supervisor.
    Start(Box<StartCommand>),
    /// Inspect the latest durable state for a run.
    Status { run_id: String },
    /// Wait for a run to start or reach a terminal state.
    Wait {
        run_id: String,
        #[arg(long = "for", value_enum, default_value = "exit")]
        condition: WaitConditionArg,
        #[arg(long, default_value = DEFAULT_WAIT_TIMEOUT)]
        timeout: String,
    },
    /// Read bounded log chunks after a monotonic cursor.
    Logs {
        run_id: String,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long, default_value_t = 200)]
        limit: usize,
        #[arg(long, value_enum, default_value = "all")]
        stream: StreamArg,
    },
    /// Stop a run's entire owned process tree. Repeated calls are safe.
    Stop {
        run_id: String,
        #[arg(long, default_value = DEFAULT_STOP_GRACE)]
        grace: String,
    },
    /// List known runs, newest first.
    List,
    /// Show port and temporary-directory leases for a run.
    Leases { run_id: String },
    /// Plan or execute garbage collection of old terminal runs.
    Gc {
        #[arg(long, default_value = "7d")]
        older_than: String,
        #[arg(long, help = "Delete eligible runs; omission is a dry run")]
        execute: bool,
    },
    /// Generate a completion script.
    Completions { shell: Shell },
    #[command(hide = true, name = "__supervise")]
    Supervise {
        #[arg(long)]
        run_dir: PathBuf,
    },
    #[command(hide = true, name = "__fixture")]
    Fixture {
        #[command(subcommand)]
        command: FixtureCommand,
    },
}

#[derive(Debug, Args)]
struct StartCommand {
    #[arg(
        long,
        help = "Child working directory; defaults to the current directory"
    )]
    cwd: Option<PathBuf>,
    #[arg(long, default_value_t = DEFAULT_LOG_BYTES)]
    max_log_bytes: u64,
    #[arg(long, default_value = DEFAULT_STARTUP_TIMEOUT)]
    startup_timeout: String,
    #[arg(long, help = "Maximum child runtime; disabled when omitted")]
    max_runtime: Option<String>,
    #[arg(long, default_value = DEFAULT_STOP_GRACE)]
    runtime_grace: String,
    #[arg(long = "ready-tcp", help = "Require a local TCP endpoint; repeatable")]
    ready_tcp: Vec<String>,
    #[arg(
        long = "ready-http",
        help = "Require a successful local HTTP response; repeatable"
    )]
    ready_http: Vec<String>,
    #[arg(long = "ready-file", help = "Require a regular file; repeatable")]
    ready_file: Vec<PathBuf>,
    #[arg(
        long = "ready-log",
        help = "Require a literal in stdout or stderr; repeatable"
    )]
    ready_log: Vec<String>,
    #[arg(long, default_value = DEFAULT_WAIT_TIMEOUT)]
    readiness_timeout: String,
    #[arg(long = "lease-port", help = "Lease a named loopback port; repeatable")]
    lease_port: Vec<String>,
    #[arg(
        long = "lease-temp-dir",
        help = "Lease a named private temporary directory; repeatable"
    )]
    lease_temp_dir: Vec<String>,
    #[arg(
        long = "ready-port",
        help = "Require a named leased port to accept TCP connections; repeatable"
    )]
    ready_port: Vec<String>,
    #[arg(
        last = true,
        required = true,
        num_args = 1..,
        help = "Program and arguments; pass them after --"
    )]
    command: Vec<OsString>,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum SchemaDocument {
    Brief,
    Run,
    Start,
    Status,
    List,
    Logs,
    Wait,
    Stop,
    Leases,
    Gc,
    Error,
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum WaitConditionArg {
    Running,
    Ready,
    Exit,
}

impl From<WaitConditionArg> for WaitCondition {
    fn from(value: WaitConditionArg) -> Self {
        match value {
            WaitConditionArg::Running => Self::Running,
            WaitConditionArg::Ready => Self::Ready,
            WaitConditionArg::Exit => Self::Exit,
        }
    }
}

#[derive(Clone, Copy, Debug, ValueEnum)]
enum StreamArg {
    All,
    Stdout,
    Stderr,
}

impl StreamArg {
    fn selected(self) -> Option<LogStream> {
        match self {
            Self::All => None,
            Self::Stdout => Some(LogStream::Stdout),
            Self::Stderr => Some(LogStream::Stderr),
        }
    }
}

#[derive(Debug, Subcommand)]
enum FixtureCommand {
    Emit {
        #[arg(long, default_value_t = 1)]
        chunks: u32,
        #[arg(long, default_value_t = 0)]
        delay_ms: u64,
        #[arg(long, default_value_t = false)]
        stderr: bool,
        #[arg(long, default_value_t = 0)]
        exit_code: i32,
    },
    Tree {
        #[arg(long)]
        marker: PathBuf,
    },
    Heartbeat {
        #[arg(long)]
        marker: PathBuf,
    },
    Serve {
        #[arg(long)]
        address: String,
        #[arg(long, default_value_t = 0)]
        startup_delay_ms: u64,
        #[arg(long, default_value_t = false)]
        http: bool,
        #[arg(long)]
        touch: Option<PathBuf>,
    },
    Touch {
        #[arg(long)]
        path: PathBuf,
        #[arg(long, default_value_t = 0)]
        delay_ms: u64,
    },
    WriteEnv {
        #[arg(long)]
        name: String,
        #[arg(long)]
        path: PathBuf,
    },
}

pub fn run() -> ExitCode {
    let cli = Cli::parse();
    let format = cli.format;
    match execute(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            render_error(format, &error);
            error.exit_code()
        }
    }
}

fn execute(cli: Cli) -> Result<(), AppError> {
    match cli.command {
        Commands::Schema { document } => emit_schema(document, cli.format),
        Commands::Completions { shell } => {
            emit_completion(shell);
            Ok(())
        }
        Commands::Supervise { run_dir } => supervisor::supervise(run_dir),
        Commands::Fixture { command } => run_fixture(command),
        command => {
            let store = Store::resolve(cli.state_dir)?;
            match command {
                Commands::Start(options) => start(&store, cli.format, *options),
                Commands::Status { run_id } => {
                    let result = StatusResult {
                        schema_version: "procherd.status.v1".to_owned(),
                        run: load_view(&store, &run_id)?,
                    };
                    render(cli.format, &result, || human_status(&result.run))
                }
                Commands::Wait {
                    run_id,
                    condition,
                    timeout,
                } => wait(
                    &store,
                    cli.format,
                    &run_id,
                    condition.into(),
                    parse_duration(&timeout)?,
                ),
                Commands::Logs {
                    run_id,
                    after,
                    limit,
                    stream,
                } => logs(&store, cli.format, &run_id, after, limit, stream.selected()),
                Commands::Stop { run_id, grace } => {
                    stop(&store, cli.format, &run_id, parse_duration(&grace)?)
                }
                Commands::List => list(&store, cli.format),
                Commands::Leases { run_id } => {
                    let view = load_view(&store, &run_id)?;
                    let result = LeasesResult {
                        schema_version: "procherd.leases.v1".to_owned(),
                        run_id,
                        supervisor_active: view.supervisor_active,
                        leases: view.state.leases,
                    };
                    render(cli.format, &result, || {
                        for lease in &result.leases.ports {
                            println!(
                                "port\t{}\t{}",
                                lease.name,
                                lease.address.as_deref().unwrap_or("pending")
                            );
                        }
                        for lease in &result.leases.temp_directories {
                            println!(
                                "temp\t{}\t{}",
                                lease.name,
                                lease.path.as_deref().map_or_else(
                                    || "pending".to_owned(),
                                    |path| path.display().to_string()
                                )
                            );
                        }
                        Ok(())
                    })
                }
                Commands::Gc {
                    older_than,
                    execute,
                } => gc(&store, cli.format, parse_duration(&older_than)?, execute),
                Commands::Schema { .. }
                | Commands::Completions { .. }
                | Commands::Supervise { .. }
                | Commands::Fixture { .. } => unreachable!(),
            }
        }
    }
}

fn start(store: &Store, format: OutputFormat, options: StartCommand) -> Result<(), AppError> {
    let StartCommand {
        cwd,
        max_log_bytes,
        startup_timeout,
        max_runtime,
        runtime_grace,
        ready_tcp,
        ready_http,
        ready_file,
        ready_log,
        readiness_timeout,
        lease_port,
        lease_temp_dir,
        ready_port,
        command,
    } = options;
    if max_log_bytes == 0 || max_log_bytes > 1024 * 1024 * 1024 {
        return Err(AppError::usage(
            "max log bytes must be between 1 and 1073741824",
        ));
    }
    let startup_timeout = parse_duration(&startup_timeout)?;
    if startup_timeout.is_zero() || startup_timeout > Duration::from_secs(60) {
        return Err(AppError::usage(
            "startup timeout must be greater than zero and at most 60s",
        ));
    }
    let max_runtime = max_runtime.as_deref().map(parse_duration).transpose()?;
    if max_runtime.is_some_and(|duration| {
        duration.is_zero() || duration > Duration::from_secs(30 * 24 * 60 * 60)
    }) {
        return Err(AppError::usage(
            "max runtime must be greater than zero and at most 30d",
        ));
    }
    let runtime_grace = parse_duration(&runtime_grace)?;
    if runtime_grace > Duration::from_secs(5 * 60) {
        return Err(AppError::usage("runtime grace must be at most 5m"));
    }
    let cwd = cwd.unwrap_or(env::current_dir()?);
    let cwd = cwd.canonicalize().map_err(|error| {
        AppError::usage(format!(
            "cannot resolve working directory {}: {error}",
            cwd.display()
        ))
    })?;
    if !cwd.is_dir() {
        return Err(AppError::usage(format!(
            "working directory is not a directory: {}",
            cwd.display()
        )));
    }
    let readiness_timeout = parse_duration(&readiness_timeout)?;
    if readiness_timeout.is_zero() || readiness_timeout > Duration::from_secs(60 * 60) {
        return Err(AppError::usage(
            "readiness timeout must be greater than zero and at most 1h",
        ));
    }
    let lease_requests = validate_requests(lease_port, lease_temp_dir)?;
    let readiness_conditions = build_conditions(
        ready_tcp,
        ready_http,
        ready_file,
        ready_log,
        ready_port,
        &lease_requests.port_names,
        &cwd,
    )?;
    let mut utf8_command = Vec::with_capacity(command.len());
    for value in command {
        utf8_command.push(value.into_string().map_err(|_| {
            AppError::usage("program and arguments must be valid UTF-8 in version 0.1")
        })?);
    }
    let program = utf8_command
        .first()
        .filter(|value| !value.is_empty())
        .cloned()
        .ok_or_else(|| AppError::usage("program must not be empty"))?;
    let args = utf8_command.into_iter().skip(1).collect::<Vec<_>>();
    let run_id = format!("run_{}", Ulid::generate());
    let owner_token = owner_token()?;
    let mut inherited_names = env::vars_os()
        .filter_map(|(name, _)| name.into_string().ok())
        .collect::<Vec<_>>();
    inherited_names.sort_unstable();
    inherited_names.dedup();
    let inherited_name_count = inherited_names.len();
    let inherited_names_sha256 = environment_names_digest(&inherited_names);
    let command = CommandSpec {
        program,
        args,
        working_directory: cwd,
        environment: EnvironmentPolicy {
            mode: EnvironmentMode::Inherit,
            inherited_name_count,
            inherited_names_sha256,
            injected_names: Vec::new(),
        },
    };
    let created_at_ms = now_ms();
    let readiness_timeout_ms = u64::try_from(readiness_timeout.as_millis()).unwrap_or(u64::MAX);
    let readiness = ReadinessState::new(readiness_conditions, readiness_timeout_ms, created_at_ms);
    let max_runtime_ms =
        max_runtime.map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX));
    let runtime_grace_ms = u64::try_from(runtime_grace.as_millis()).unwrap_or(u64::MAX);
    let state = RunState {
        schema_version: RUN_SCHEMA_VERSION.to_owned(),
        run_id: run_id.clone(),
        status: RunStatus::Created,
        created_at_ms,
        updated_at_ms: created_at_ms,
        command: command.clone(),
        process: ProcessInfo {
            supervisor_pid: None,
            process_pid: None,
            tree_control: platform_tree_control(),
        },
        exit: None,
        failure: None,
        readiness: readiness.clone(),
        leases: LeaseState::requested(
            &lease_requests.port_names,
            &lease_requests.temp_directory_names,
        ),
        limits: RunLimits {
            max_runtime_ms,
            runtime_deadline_at_ms: None,
            runtime_grace_ms,
            runtime_limit_triggered_at_ms: None,
        },
        cleanup: CleanupInfo::for_platform(),
        logs: crate::model::LogSummary::new(max_log_bytes),
    };
    let spec = SupervisorSpec {
        schema_version: "procherd.supervisor-spec.v1".to_owned(),
        run_id: run_id.clone(),
        owner_token: owner_token.clone(),
        command,
        max_log_bytes,
        readiness,
        lease_requests,
        max_runtime_ms,
        runtime_grace_ms,
    };
    let run_dir = store.create_run(&state, &spec, &owner_token)?;
    if let Err(error) = supervisor::launch(&run_dir) {
        let mut failed = state;
        failed.status = RunStatus::Failed;
        failed.updated_at_ms = now_ms();
        failed.exit = Some(crate::model::ExitInfo {
            code: None,
            signal: None,
            reason: crate::model::ExitReason::SupervisorFailed,
            finished_at_ms: now_ms(),
        });
        failed.failure = Some(crate::model::FailureInfo {
            kind: error.kind.to_owned(),
            message: error.message.clone(),
        });
        let _ = atomic_write_json(&run_dir.join("state.json"), &failed);
        return Err(AppError::operational(
            "supervisor_launch",
            format!("run {run_id} could not start its supervisor: {error}"),
        ));
    }

    let deadline = Instant::now() + startup_timeout;
    let mut active_seen = false;
    loop {
        let state = store.read_state(&run_id)?;
        let active = store.supervisor_active(&run_id)?;
        active_seen |= active;
        if !matches!(state.status, RunStatus::Created | RunStatus::Starting) {
            if state.status.is_terminal() && active {
                if Instant::now() >= deadline {
                    return Err(AppError::timeout(format!(
                        "run {run_id} reached terminal state but its supervisor remained active"
                    )));
                }
                thread::sleep(Duration::from_millis(10));
                continue;
            }
            if state.status == RunStatus::Failed {
                let detail = state.failure.as_ref().map_or_else(
                    || "the supervisor did not provide a diagnostic".to_owned(),
                    |failure| format!("{}: {}", failure.kind, failure.message),
                );
                return Err(AppError::operational(
                    "start_failed",
                    format!("run {run_id} failed to start: {detail}"),
                ));
            }
            if !active {
                return Err(AppError::operational(
                    "supervisor_exit",
                    format!("run {run_id} supervisor exited during startup"),
                ));
            }
            let result = StartResult {
                schema_version: "procherd.start.v1".to_owned(),
                run: RunView::new(state, active),
            };
            return render(format, &result, || {
                println!(
                    "{} {}",
                    result.run.state.run_id,
                    status_name(result.run.state.status)
                );
                Ok(())
            });
        }
        if active_seen && !active {
            return Err(AppError::operational(
                "supervisor_exit",
                format!("run {run_id} supervisor exited during startup"),
            ));
        }
        if Instant::now() >= deadline {
            let request = StopRequest {
                schema_version: "procherd.stop-request.v1".to_owned(),
                run_id: run_id.clone(),
                owner_token,
                requested_at_ms: now_ms(),
                grace_ms: 0,
            };
            let _ = store.write_stop_request(&request);
            return Err(AppError::timeout(format!(
                "run {run_id} did not finish starting within {} ms",
                startup_timeout.as_millis()
            )));
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn load_view(store: &Store, run_id: &str) -> Result<RunView, AppError> {
    let state = store.read_state(run_id)?;
    if state.schema_version != RUN_SCHEMA_VERSION {
        return Err(AppError::integrity(format!(
            "unsupported run schema {}",
            state.schema_version
        )));
    }
    let active = store.supervisor_active(run_id)?;
    Ok(RunView::new(state, active))
}

fn wait(
    store: &Store,
    format: OutputFormat,
    run_id: &str,
    condition: WaitCondition,
    timeout: Duration,
) -> Result<(), AppError> {
    if timeout.is_zero() || timeout > Duration::from_secs(24 * 60 * 60) {
        return Err(AppError::usage(
            "wait timeout must be greater than zero and at most 24h",
        ));
    }
    let deadline = Instant::now() + timeout;
    loop {
        let view = load_view(store, run_id)?;
        if matches!(view.observed_status, ObservedStatus::Orphaned) {
            return Err(AppError::operational(
                "orphaned",
                format!("run {run_id} has live durable state but no active supervisor"),
            ));
        }
        let satisfied = match condition {
            WaitCondition::Running => view.state.process.process_pid.is_some(),
            WaitCondition::Ready => match view.state.readiness.status {
                ReadinessStatus::Ready => true,
                ReadinessStatus::NotConfigured => {
                    return Err(AppError::usage(format!(
                        "run {run_id} has no readiness conditions"
                    )));
                }
                ReadinessStatus::TimedOut | ReadinessStatus::Failed => {
                    return Err(AppError::operational(
                        "readiness_failed",
                        format!(
                            "run {run_id} readiness ended as {:?}: {}",
                            view.state.readiness.status,
                            view.state
                                .readiness
                                .failure_reason
                                .as_deref()
                                .unwrap_or("unknown")
                        ),
                    ));
                }
                ReadinessStatus::Pending => {
                    if view.state.status.is_terminal() {
                        return Err(AppError::operational(
                            "readiness_failed",
                            format!("run {run_id} exited before it became ready"),
                        ));
                    }
                    false
                }
            },
            WaitCondition::Exit => view.state.status.is_terminal() && !view.supervisor_active,
        };
        if satisfied {
            let result = WaitResult {
                schema_version: "procherd.wait.v1".to_owned(),
                condition,
                run: view,
            };
            return render(format, &result, || human_status(&result.run));
        }
        if Instant::now() >= deadline {
            return Err(AppError::timeout(format!(
                "run {run_id} did not reach {} within {} ms",
                wait_condition_name(condition),
                timeout.as_millis()
            )));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn logs(
    store: &Store,
    format: OutputFormat,
    run_id: &str,
    after: u64,
    limit: usize,
    stream: Option<LogStream>,
) -> Result<(), AppError> {
    let state = store.read_state(run_id)?;
    let run_dir = store.run_dir(run_id)?;
    let result = read_logs(&run_dir, &state, after, limit, stream)?;
    render(format, &result, || {
        for record in &result.records {
            let bytes = decode_record(record)?;
            let label = match record.stream {
                LogStream::Stdout => "stdout",
                LogStream::Stderr => "stderr",
            };
            let text = String::from_utf8_lossy(&bytes);
            print!("[{} {label}] {text}", record.cursor);
            if !text.ends_with('\n') {
                println!();
            }
        }
        if result.dropped_bytes > 0 {
            eprintln!(
                "procherd: {} log bytes were dropped after the configured bound",
                result.dropped_bytes
            );
        }
        Ok::<(), AppError>(())
    })
}

fn stop(
    store: &Store,
    format: OutputFormat,
    run_id: &str,
    grace: Duration,
) -> Result<(), AppError> {
    if grace > Duration::from_secs(5 * 60) {
        return Err(AppError::usage("stop grace must be at most 5m"));
    }
    let mut view = load_view(store, run_id)?;
    if view.state.status.is_terminal() {
        let deadline = Instant::now() + Duration::from_secs(5);
        while view.supervisor_active {
            if Instant::now() >= deadline {
                return Err(AppError::timeout(format!(
                    "run {run_id} reached terminal state but its supervisor remained active"
                )));
            }
            thread::sleep(Duration::from_millis(10));
            view = load_view(store, run_id)?;
        }
        let result = StopResult {
            schema_version: "procherd.stop.v1".to_owned(),
            already_terminal: true,
            run: view,
        };
        return render(format, &result, || human_status(&result.run));
    }
    if !view.supervisor_active {
        return Err(AppError::operational(
            "orphaned",
            format!("run {run_id} cannot be stopped because its supervisor is not active"),
        ));
    }
    let owner_token = store.read_owner_token(run_id)?;
    let grace_ms = u64::try_from(grace.as_millis()).unwrap_or(u64::MAX);
    let request = StopRequest {
        schema_version: "procherd.stop-request.v1".to_owned(),
        run_id: run_id.to_owned(),
        owner_token,
        requested_at_ms: now_ms(),
        grace_ms,
    };
    store.write_stop_request(&request)?;
    let deadline = Instant::now() + grace + Duration::from_secs(5);
    loop {
        let view = load_view(store, run_id)?;
        if view.state.status.is_terminal() && !view.supervisor_active {
            let result = StopResult {
                schema_version: "procherd.stop.v1".to_owned(),
                already_terminal: false,
                run: view,
            };
            return render(format, &result, || human_status(&result.run));
        }
        if !view.supervisor_active {
            return Err(AppError::operational(
                "orphaned",
                format!("run {run_id} supervisor exited before persisting terminal state"),
            ));
        }
        if Instant::now() >= deadline {
            return Err(AppError::timeout(format!(
                "run {run_id} did not stop within the grace period"
            )));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn list(store: &Store, format: OutputFormat) -> Result<(), AppError> {
    let mut runs = Vec::new();
    for run_id in store.list_run_ids()? {
        runs.push(load_view(store, &run_id)?);
    }
    let result = ListResult {
        schema_version: "procherd.list.v1".to_owned(),
        runs,
    };
    render(format, &result, || {
        for run in &result.runs {
            println!(
                "{}\t{}\t{}",
                run.state.run_id,
                status_name(run.state.status),
                run.state.command.program
            );
        }
        Ok(())
    })
}

fn gc(
    store: &Store,
    format: OutputFormat,
    older_than: Duration,
    execute: bool,
) -> Result<(), AppError> {
    let older_than_ms = u64::try_from(older_than.as_millis()).unwrap_or(u64::MAX);
    let observed_at_ms = now_ms();
    let mut entries = Vec::new();
    for run_id in store.list_run_ids()? {
        let view = load_view(store, &run_id)?;
        let age_ms = observed_at_ms.saturating_sub(view.state.updated_at_ms);
        let (eligible, reason) = if view.supervisor_active {
            (false, "supervisor_active")
        } else if !view.state.status.is_terminal() {
            (false, "non_terminal_or_orphaned")
        } else if age_ms < older_than_ms {
            (false, "younger_than_threshold")
        } else {
            (true, "terminal_and_inactive")
        };
        let mut entry = GcEntry {
            run_id,
            status: view.state.status,
            age_ms,
            eligible,
            reason: reason.to_owned(),
            deleted: false,
        };
        if execute && eligible {
            store.remove_terminal_run(&entry.run_id)?;
            entry.deleted = true;
        }
        entries.push(entry);
    }
    let result = GcResult {
        schema_version: "procherd.gc.v1".to_owned(),
        execute,
        older_than_ms,
        entries,
    };
    render(format, &result, || {
        for entry in &result.entries {
            println!(
                "{}\t{}\t{}",
                if entry.deleted {
                    "deleted"
                } else if entry.eligible {
                    "eligible"
                } else {
                    "kept"
                },
                entry.run_id,
                entry.reason
            );
        }
        Ok(())
    })
}

fn emit_schema(document: SchemaDocument, format: OutputFormat) -> Result<(), AppError> {
    let value = match document {
        SchemaDocument::Brief => json!({
            "schema_version": "procherd.brief.v1",
            "procherd_version": env!("CARGO_PKG_VERSION"),
            "commands": ["schema", "start", "status", "wait", "logs", "stop", "list", "leases", "gc", "completions"],
            "run_states": ["created", "starting", "running", "stopping", "exited", "failed", "stopped"],
            "readiness_states": ["not_configured", "pending", "ready", "timed_out", "failed"],
            "readiness_conditions": ["tcp", "http", "file", "log", "port_lease"],
            "lease_placeholders": ["{port:NAME}", "{temp:NAME}"],
            "tree_control": {
                "unix": "process_group",
                "windows": "job_object"
            },
            "log_contract": {
                "encoding": "base64",
                "cursor": "monotonic per run",
                "retention": "hard byte bound with explicit dropped_bytes"
            },
            "exit_codes": {
                "0": "success",
                "1": "operational failure",
                "2": "usage error",
                "3": "timeout",
                "4": "run not found",
                "5": "state or log integrity failure"
            },
            "security": {
                "shell_interpretation": false,
                "mutation": "run-scoped owner token stored in the private run directory",
                "environment": "inherited; name count and digest recorded, values never persisted by ProcHerd"
            }
        }),
        SchemaDocument::Run => serde_json::to_value(schema_for!(RunState))?,
        SchemaDocument::Start => serde_json::to_value(schema_for!(StartResult))?,
        SchemaDocument::Status => serde_json::to_value(schema_for!(StatusResult))?,
        SchemaDocument::List => serde_json::to_value(schema_for!(ListResult))?,
        SchemaDocument::Logs => serde_json::to_value(schema_for!(crate::model::LogsResult))?,
        SchemaDocument::Wait => serde_json::to_value(schema_for!(WaitResult))?,
        SchemaDocument::Stop => serde_json::to_value(schema_for!(StopResult))?,
        SchemaDocument::Leases => serde_json::to_value(schema_for!(LeasesResult))?,
        SchemaDocument::Gc => serde_json::to_value(schema_for!(GcResult))?,
        SchemaDocument::Error => {
            serde_json::to_value(schema_for!(crate::error::ErrorDocument<'static>))?
        }
    };
    match format {
        OutputFormat::Human | OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
        OutputFormat::Ndjson => println!("{}", serde_json::to_string(&value)?),
    }
    Ok(())
}

fn emit_completion<G: Generator>(generator: G) {
    let mut command = Cli::command();
    generate(generator, &mut command, "procherd", &mut io::stdout());
}

fn render<T, F>(format: OutputFormat, value: &T, human: F) -> Result<(), AppError>
where
    T: Serialize,
    F: FnOnce() -> Result<(), AppError>,
{
    match format {
        OutputFormat::Human => human(),
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(value)?);
            Ok(())
        }
        OutputFormat::Ndjson => {
            println!("{}", serde_json::to_string(value)?);
            Ok(())
        }
    }
}

fn render_error(format: OutputFormat, error: &AppError) {
    match format {
        OutputFormat::Human => eprintln!("procherd: {}", error.message),
        OutputFormat::Json => {
            let document = ErrorDocument::from(error);
            match serde_json::to_string_pretty(&document) {
                Ok(json) => eprintln!("{json}"),
                Err(_) => eprintln!("procherd: {}", error.message),
            }
        }
        OutputFormat::Ndjson => {
            let document = ErrorDocument::from(error);
            match serde_json::to_string(&document) {
                Ok(json) => eprintln!("{json}"),
                Err(_) => eprintln!("procherd: {}", error.message),
            }
        }
    }
}

fn human_status(view: &RunView) -> Result<(), AppError> {
    println!(
        "{} {} readiness={} supervisor={}",
        view.state.run_id,
        status_name(view.state.status),
        readiness_name(view.state.readiness.status),
        if view.supervisor_active {
            "active"
        } else {
            "inactive"
        }
    );
    Ok(())
}

fn status_name(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Created => "created",
        RunStatus::Starting => "starting",
        RunStatus::Running => "running",
        RunStatus::Stopping => "stopping",
        RunStatus::Exited => "exited",
        RunStatus::Failed => "failed",
        RunStatus::Stopped => "stopped",
    }
}

fn readiness_name(status: ReadinessStatus) -> &'static str {
    match status {
        ReadinessStatus::NotConfigured => "not-configured",
        ReadinessStatus::Pending => "pending",
        ReadinessStatus::Ready => "ready",
        ReadinessStatus::TimedOut => "timed-out",
        ReadinessStatus::Failed => "failed",
    }
}

fn wait_condition_name(condition: WaitCondition) -> &'static str {
    match condition {
        WaitCondition::Running => "running",
        WaitCondition::Ready => "ready",
        WaitCondition::Exit => "exit",
    }
}

fn platform_tree_control() -> TreeControl {
    #[cfg(unix)]
    {
        TreeControl::UnixProcessGroup
    }
    #[cfg(windows)]
    {
        TreeControl::WindowsJobObject
    }
}

fn owner_token() -> Result<String, AppError> {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).map_err(|error| {
        AppError::operational(
            "randomness",
            format!("cannot create run owner token: {error}"),
        )
    })?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn environment_names_digest(names: &[String]) -> String {
    let mut hasher = Sha256::new();
    for name in names {
        hasher.update(name.as_bytes());
        hasher.update([0]);
    }
    format!("{:x}", hasher.finalize())
}

pub fn parse_duration(value: &str) -> Result<Duration, AppError> {
    let (number, multiplier) = if let Some(number) = value.strip_suffix("ms") {
        (number, 1_u64)
    } else if let Some(number) = value.strip_suffix('s') {
        (number, 1_000)
    } else if let Some(number) = value.strip_suffix('m') {
        (number, 60_000)
    } else if let Some(number) = value.strip_suffix('h') {
        (number, 3_600_000)
    } else if let Some(number) = value.strip_suffix('d') {
        (number, 86_400_000)
    } else {
        return Err(AppError::usage(
            "duration must have an ms, s, m, h, or d suffix",
        ));
    };
    let number = number
        .parse::<u64>()
        .map_err(|_| AppError::usage(format!("invalid duration: {value}")))?;
    let millis = number
        .checked_mul(multiplier)
        .ok_or_else(|| AppError::usage("duration is too large"))?;
    Ok(Duration::from_millis(millis))
}

fn run_fixture(command: FixtureCommand) -> Result<(), AppError> {
    match command {
        FixtureCommand::Emit {
            chunks,
            delay_ms,
            stderr,
            exit_code,
        } => {
            for index in 0..chunks {
                if stderr {
                    eprintln!("fixture-{index}");
                } else {
                    println!("fixture-{index}");
                }
                if delay_ms > 0 {
                    thread::sleep(Duration::from_millis(delay_ms));
                }
            }
            std::process::exit(exit_code);
        }
        FixtureCommand::Tree { marker } => {
            let child = Command::new(env::current_exe()?)
                .arg("__fixture")
                .arg("heartbeat")
                .arg("--marker")
                .arg(marker)
                .spawn()?;
            let _child = child;
            loop {
                thread::sleep(Duration::from_secs(60));
            }
        }
        FixtureCommand::Heartbeat { marker } => loop {
            let mut file = OpenOptions::new().create(true).append(true).open(&marker)?;
            writeln!(file, "{}", now_ms())?;
            file.flush()?;
            thread::sleep(Duration::from_millis(25));
        },
        FixtureCommand::Serve {
            address,
            startup_delay_ms,
            http,
            touch,
        } => {
            thread::sleep(Duration::from_millis(startup_delay_ms));
            let listener = TcpListener::bind(&address)?;
            if let Some(path) = touch {
                let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
                file.write_all(b"ready\n")?;
                file.sync_all()?;
            }
            println!("listening {address}");
            for stream in listener.incoming() {
                let mut stream = stream?;
                if http {
                    let _ = stream.set_read_timeout(Some(Duration::from_millis(150)));
                    let _ = stream.set_write_timeout(Some(Duration::from_millis(150)));
                    let mut request = [0_u8; 1024];
                    if stream.read(&mut request).is_ok() {
                        stream.write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                        )?;
                    }
                }
            }
            Ok(())
        }
        FixtureCommand::Touch { path, delay_ms } => {
            thread::sleep(Duration::from_millis(delay_ms));
            let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
            file.write_all(b"ready\n")?;
            file.sync_all()?;
            loop {
                thread::sleep(Duration::from_secs(60));
            }
        }
        FixtureCommand::WriteEnv { name, path } => {
            let value = env::var_os(&name)
                .ok_or_else(|| AppError::operational("fixture", format!("{name} is not set")))?;
            let mut file = OpenOptions::new().create_new(true).write(true).open(path)?;
            file.write_all(value.to_string_lossy().as_bytes())?;
            file.sync_all()?;
            loop {
                thread::sleep(Duration::from_secs(60));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::parse_duration;
    use std::time::Duration;

    #[test]
    fn parses_bounded_duration_units() {
        assert_eq!(parse_duration("25ms").unwrap(), Duration::from_millis(25));
        assert_eq!(parse_duration("3s").unwrap(), Duration::from_secs(3));
        assert_eq!(parse_duration("2m").unwrap(), Duration::from_secs(120));
        assert_eq!(parse_duration("1h").unwrap(), Duration::from_secs(3600));
        assert_eq!(parse_duration("1d").unwrap(), Duration::from_secs(86_400));
        assert!(parse_duration("10").is_err());
        assert!(parse_duration("xs").is_err());
    }
}
