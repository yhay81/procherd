use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const RUN_SCHEMA_VERSION: &str = "procherd.run.v1";
pub const LOG_RECORD_SCHEMA_VERSION: &str = "procherd.log-record.v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Created,
    Starting,
    Running,
    Stopping,
    Exited,
    Failed,
    Stopped,
}

impl RunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Exited | Self::Failed | Self::Stopped)
    }

    pub fn is_live(self) -> bool {
        matches!(
            self,
            Self::Created | Self::Starting | Self::Running | Self::Stopping
        )
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CommandSpec {
    pub program: String,
    pub args: Vec<String>,
    pub working_directory: PathBuf,
    pub environment: EnvironmentPolicy,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentPolicy {
    pub mode: EnvironmentMode,
    pub inherited_name_count: usize,
    pub inherited_names_sha256: String,
    #[serde(default)]
    pub injected_names: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentMode {
    Inherit,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessInfo {
    pub supervisor_pid: Option<u32>,
    pub process_pid: Option<u32>,
    pub tree_control: TreeControl,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TreeControl {
    UnixProcessGroup,
    WindowsJobObject,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExitInfo {
    pub code: Option<i32>,
    pub signal: Option<i32>,
    pub reason: ExitReason,
    pub finished_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitReason {
    ProcessExited,
    SpawnFailed,
    StopRequested,
    RuntimeLimit,
    SupervisorFailed,
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CleanupInfo {
    pub requested_at_ms: Option<u64>,
    pub completed_at_ms: Option<u64>,
    pub grace_ms: Option<u64>,
    pub graceful_signal_supported: bool,
    pub force_used: bool,
    pub descendant_cleanup_triggered: bool,
}

impl CleanupInfo {
    pub fn for_platform() -> Self {
        Self {
            graceful_signal_supported: cfg!(unix),
            ..Self::default()
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LogSummary {
    pub next_cursor: u64,
    pub captured_bytes: u64,
    pub dropped_bytes: u64,
    pub max_bytes: u64,
    pub stdout_sha256: Option<String>,
    pub stderr_sha256: Option<String>,
}

impl LogSummary {
    pub fn new(max_bytes: u64) -> Self {
        Self {
            next_cursor: 1,
            captured_bytes: 0,
            dropped_bytes: 0,
            max_bytes,
            stdout_sha256: None,
            stderr_sha256: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunState {
    pub schema_version: String,
    pub run_id: String,
    pub status: RunStatus,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub command: CommandSpec,
    pub process: ProcessInfo,
    pub exit: Option<ExitInfo>,
    #[serde(default)]
    pub failure: Option<FailureInfo>,
    pub readiness: ReadinessState,
    pub leases: LeaseState,
    pub limits: RunLimits,
    pub cleanup: CleanupInfo,
    pub logs: LogSummary,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FailureInfo {
    pub kind: String,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessState {
    pub status: ReadinessStatus,
    pub timeout_ms: u64,
    pub deadline_at_ms: Option<u64>,
    pub ready_at_ms: Option<u64>,
    pub failure_reason: Option<String>,
    pub checks: Vec<ReadinessCheck>,
}

impl ReadinessState {
    pub fn new(conditions: Vec<ReadinessCondition>, timeout_ms: u64, created_at_ms: u64) -> Self {
        let configured = !conditions.is_empty();
        Self {
            status: if configured {
                ReadinessStatus::Pending
            } else {
                ReadinessStatus::NotConfigured
            },
            timeout_ms,
            deadline_at_ms: configured.then(|| created_at_ms.saturating_add(timeout_ms)),
            ready_at_ms: None,
            failure_reason: None,
            checks: conditions
                .into_iter()
                .enumerate()
                .map(|(index, condition)| ReadinessCheck {
                    id: format!("ready-{}", index + 1),
                    condition,
                    ready_at_ms: None,
                    evidence: None,
                })
                .collect(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessStatus {
    NotConfigured,
    Pending,
    Ready,
    TimedOut,
    Failed,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReadinessCheck {
    pub id: String,
    pub condition: ReadinessCondition,
    pub ready_at_ms: Option<u64>,
    pub evidence: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReadinessCondition {
    Tcp { address: String },
    Http { url: String },
    File { path: PathBuf },
    Log { literal: String },
    PortLease { name: String },
}

#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LeaseState {
    pub ports: Vec<PortLease>,
    pub temp_directories: Vec<TempDirectoryLease>,
}

impl LeaseState {
    pub fn requested(port_names: &[String], temp_names: &[String]) -> Self {
        Self {
            ports: port_names
                .iter()
                .map(|name| PortLease {
                    name: name.clone(),
                    address: None,
                    port: None,
                    acquired_at_ms: None,
                    handoff_at_ms: None,
                    handoff_gap_ms: None,
                    released_at_ms: None,
                    guarantee: PortLeaseGuarantee::CoordinatedBestEffort,
                })
                .collect(),
            temp_directories: temp_names
                .iter()
                .map(|name| TempDirectoryLease {
                    name: name.clone(),
                    path: None,
                    acquired_at_ms: None,
                    released_at_ms: None,
                    retained_after_release: true,
                })
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PortLease {
    pub name: String,
    pub address: Option<String>,
    pub port: Option<u16>,
    pub acquired_at_ms: Option<u64>,
    pub handoff_at_ms: Option<u64>,
    pub handoff_gap_ms: Option<u64>,
    pub released_at_ms: Option<u64>,
    pub guarantee: PortLeaseGuarantee,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PortLeaseGuarantee {
    CoordinatedBestEffort,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TempDirectoryLease {
    pub name: String,
    pub path: Option<PathBuf>,
    pub acquired_at_ms: Option<u64>,
    pub released_at_ms: Option<u64>,
    pub retained_after_release: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LeaseRequests {
    pub port_names: Vec<String>,
    pub temp_directory_names: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunLimits {
    pub max_runtime_ms: Option<u64>,
    pub runtime_deadline_at_ms: Option<u64>,
    pub runtime_grace_ms: u64,
    pub runtime_limit_triggered_at_ms: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct StartResult {
    pub schema_version: String,
    pub run: RunView,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct StatusResult {
    pub schema_version: String,
    pub run: RunView,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct RunView {
    #[serde(flatten)]
    pub state: RunState,
    pub supervisor_active: bool,
    pub observed_status: ObservedStatus,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservedStatus {
    Consistent,
    Orphaned,
}

impl RunView {
    pub fn new(state: RunState, supervisor_active: bool) -> Self {
        let observed_status = if state.status.is_live() && !supervisor_active {
            ObservedStatus::Orphaned
        } else {
            ObservedStatus::Consistent
        };
        Self {
            state,
            supervisor_active,
            observed_status,
        }
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SupervisorSpec {
    pub schema_version: String,
    pub run_id: String,
    pub owner_token: String,
    pub command: CommandSpec,
    pub max_log_bytes: u64,
    pub readiness: ReadinessState,
    pub lease_requests: LeaseRequests,
    pub max_runtime_ms: Option<u64>,
    pub runtime_grace_ms: u64,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StopRequest {
    pub schema_version: String,
    pub run_id: String,
    pub owner_token: String,
    pub requested_at_ms: u64,
    pub grace_ms: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    Stdout,
    Stderr,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LogRecord {
    pub schema_version: String,
    pub cursor: u64,
    pub timestamp_ms: u64,
    pub stream: LogStream,
    pub encoding: LogEncoding,
    pub data: String,
    pub byte_count: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogEncoding {
    Base64,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct LogsResult {
    pub schema_version: String,
    pub run_id: String,
    pub after_cursor: u64,
    pub records: Vec<LogRecord>,
    pub next_after_cursor: u64,
    pub has_more: bool,
    pub captured_bytes: u64,
    pub dropped_bytes: u64,
    pub terminal: bool,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct WaitResult {
    pub schema_version: String,
    pub condition: WaitCondition,
    pub run: RunView,
}

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitCondition {
    Running,
    Ready,
    Exit,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct StopResult {
    pub schema_version: String,
    pub already_terminal: bool,
    pub run: RunView,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct ListResult {
    pub schema_version: String,
    pub runs: Vec<RunView>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct LeasesResult {
    pub schema_version: String,
    pub run_id: String,
    pub supervisor_active: bool,
    pub leases: LeaseState,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct GcResult {
    pub schema_version: String,
    pub execute: bool,
    pub older_than_ms: u64,
    pub entries: Vec<GcEntry>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
pub struct GcEntry {
    pub run_id: String,
    pub status: RunStatus,
    pub age_ms: u64,
    pub eligible: bool,
    pub reason: String,
    pub deleted: bool,
}
