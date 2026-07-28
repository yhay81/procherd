use std::{
    fs,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Command, Output},
    thread,
    time::{Duration, Instant},
};

use procherd::model::{
    ExitReason, GcResult, LeasesResult, LogsResult, ReadinessStatus, RunStatus, StartResult,
    StatusResult, StopResult, WaitResult,
};
use serde::de::DeserializeOwned;
use tempfile::TempDir;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_procherd"))
}

fn invoke(state_dir: &Path, arguments: &[&str]) -> Output {
    let started = Instant::now();
    eprintln!("invoke ProcHerd: {}", arguments.join(" "));
    let output = Command::new(binary())
        .arg("--state-dir")
        .arg(state_dir)
        .arg("--format")
        .arg("json")
        .args(arguments)
        .output()
        .expect("run procherd");
    eprintln!(
        "completed ProcHerd after {:?} with {:?}: {}",
        started.elapsed(),
        output.status.code(),
        arguments.first().copied().unwrap_or("<none>")
    );
    output
}

fn parse_success<T: DeserializeOwned>(output: Output) -> T {
    assert!(
        output.status.success(),
        "command failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("valid JSON response")
}

fn start_fixture(state_dir: &Path, fixture_arguments: &[&str]) -> StartResult {
    let binary = binary();
    let binary = binary.to_str().expect("UTF-8 binary path");
    let mut arguments = vec!["start", "--", binary, "__fixture"];
    arguments.extend_from_slice(fixture_arguments);
    parse_success(invoke(state_dir, &arguments))
}

#[test]
fn every_public_result_contract_has_a_json_schema() {
    let temp = TempDir::new().unwrap();
    for document in [
        "brief", "run", "start", "status", "list", "logs", "wait", "stop", "leases", "gc", "error",
    ] {
        let output = invoke(temp.path(), &["schema", "--document", document]);
        assert!(
            output.status.success(),
            "schema {document} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("valid JSON schema");
        assert!(value.is_object(), "schema {document} is not an object");
    }
}

#[test]
fn fast_terminal_starts_settle_as_success_instead_of_supervisor_failure() {
    let temp = TempDir::new().unwrap();
    for _ in 0..10 {
        let start = start_fixture(temp.path(), &["emit"]);
        let run_id = &start.run.state.run_id;
        let finished: WaitResult = parse_success(invoke(
            temp.path(),
            &["wait", run_id, "--for", "exit", "--timeout", "5s"],
        ));
        assert_eq!(finished.run.state.status, RunStatus::Exited);
        assert!(!finished.run.supervisor_active);
    }
}

#[test]
fn detached_run_can_be_reacquired_and_logs_are_cursor_bound() {
    let temp = TempDir::new().unwrap();
    // A pipe may coalesce separate writes into one read. Emit more than the
    // supervisor's 8 KiB read buffer so pagination always spans log records.
    let start = start_fixture(temp.path(), &["emit", "--chunks", "1000"]);
    let run_id = &start.run.state.run_id;
    assert!(matches!(
        start.run.state.status,
        RunStatus::Running | RunStatus::Exited
    ));

    let wait: WaitResult = parse_success(invoke(
        temp.path(),
        &["wait", run_id, "--for", "exit", "--timeout", "5s"],
    ));
    assert_eq!(wait.run.state.status, RunStatus::Exited);
    assert_eq!(wait.run.state.exit.as_ref().unwrap().code, Some(0));

    let logs: LogsResult = parse_success(invoke(temp.path(), &["logs", run_id, "--limit", "1"]));
    assert_eq!(logs.records.len(), 1);
    assert_eq!(logs.next_after_cursor, logs.records[0].cursor);
    assert!(logs.has_more);
    assert_eq!(logs.dropped_bytes, 0);

    let next = logs.next_after_cursor.to_string();
    let remaining: LogsResult = parse_success(invoke(
        temp.path(),
        &["logs", run_id, "--after", &next, "--limit", "10"],
    ));
    assert!(!remaining.records.is_empty());
    assert!(
        remaining
            .records
            .iter()
            .all(|record| record.cursor > logs.next_after_cursor)
    );

    wait_until(Duration::from_secs(5), || {
        let output = invoke(temp.path(), &["status", run_id]);
        output.status.success()
            && serde_json::from_slice::<StatusResult>(&output.stdout)
                .is_ok_and(|status| !status.run.supervisor_active)
    });
    let status: StatusResult = parse_success(invoke(temp.path(), &["status", run_id]));
    assert_eq!(status.run.state.status, RunStatus::Exited);
}

#[test]
fn log_limit_drops_bytes_explicitly_without_blocking_the_child() {
    let temp = TempDir::new().unwrap();
    let binary = binary();
    let binary = binary.to_str().unwrap();
    let start: StartResult = parse_success(invoke(
        temp.path(),
        &[
            "start",
            "--max-log-bytes",
            "15",
            "--",
            binary,
            "__fixture",
            "emit",
            "--chunks",
            "4",
        ],
    ));
    let run_id = &start.run.state.run_id;
    let wait: WaitResult = parse_success(invoke(
        temp.path(),
        &["wait", run_id, "--for", "exit", "--timeout", "5s"],
    ));
    assert_eq!(wait.run.state.logs.captured_bytes, 15);
    assert_eq!(wait.run.state.logs.dropped_bytes, 25);
    assert!(wait.run.state.logs.stdout_sha256.is_some());
    assert!(wait.run.state.logs.stderr_sha256.is_some());
}

#[test]
fn stop_cleans_up_descendants_and_is_idempotent() {
    let temp = TempDir::new().unwrap();
    let marker = temp.path().join("heartbeat.log");
    let marker_text = marker.to_str().unwrap();
    let start = start_fixture(temp.path(), &["tree", "--marker", marker_text]);
    let run_id = &start.run.state.run_id;

    wait_until(Duration::from_secs(5), || {
        fs::metadata(&marker).is_ok_and(|metadata| metadata.len() > 0)
    });

    let stopped: StopResult =
        parse_success(invoke(temp.path(), &["stop", run_id, "--grace", "100ms"]));
    assert_eq!(stopped.run.state.status, RunStatus::Stopped);
    assert!(stopped.run.state.cleanup.descendant_cleanup_triggered);
    assert!(!stopped.run.supervisor_active);

    let settled_size = fs::metadata(&marker).unwrap().len();
    thread::sleep(Duration::from_millis(300));
    assert_eq!(fs::metadata(&marker).unwrap().len(), settled_size);

    let again: StopResult =
        parse_success(invoke(temp.path(), &["stop", run_id, "--grace", "100ms"]));
    assert!(again.already_terminal);
    assert_eq!(again.run.state.status, RunStatus::Stopped);
}

#[test]
fn spawn_failure_is_structured_and_persisted() {
    let temp = TempDir::new().unwrap();
    let output = invoke(
        temp.path(),
        &["start", "--", "definitely-not-a-real-procherd-program"],
    );
    assert_eq!(output.status.code(), Some(1));
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["kind"], "start_failed");
    assert_eq!(error["error"]["exit_code"], 1);

    let run_id = fs::read_dir(temp.path())
        .unwrap()
        .filter_map(Result::ok)
        .find_map(|entry| {
            entry
                .file_name()
                .to_str()
                .filter(|name| name.starts_with("run_"))
                .map(str::to_owned)
        })
        .expect("durable failed run");
    let status: StatusResult = parse_success(invoke(temp.path(), &["status", &run_id]));
    assert_eq!(status.run.state.status, RunStatus::Failed);
    assert_eq!(
        status.run.state.failure.as_ref().unwrap().kind,
        "process_spawn"
    );
    assert!(!status.run.supervisor_active);
}

#[test]
fn all_readiness_conditions_are_evidenced_before_ready() {
    let temp = TempDir::new().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    drop(listener);
    let url = format!("http://{address}/health");
    let marker = temp.path().join("ready.flag");
    let marker_text = marker.to_str().unwrap();
    let binary = binary();
    let binary = binary.to_str().unwrap();

    let start: StartResult = parse_success(invoke(
        temp.path(),
        &[
            "start",
            "--ready-tcp",
            &address,
            "--ready-http",
            &url,
            "--ready-file",
            marker_text,
            "--ready-log",
            "listening",
            "--readiness-timeout",
            "5s",
            "--",
            binary,
            "__fixture",
            "serve",
            "--address",
            &address,
            "--startup-delay-ms",
            "100",
            "--http",
            "--touch",
            marker_text,
        ],
    ));
    let run_id = &start.run.state.run_id;
    let ready: WaitResult = parse_success(invoke(
        temp.path(),
        &["wait", run_id, "--for", "ready", "--timeout", "5s"],
    ));
    assert_eq!(ready.run.state.readiness.status, ReadinessStatus::Ready);
    assert_eq!(ready.run.state.readiness.checks.len(), 4);
    assert!(
        ready
            .run
            .state
            .readiness
            .checks
            .iter()
            .all(|check| check.ready_at_ms.is_some() && check.evidence.is_some())
    );

    let stopped: StopResult =
        parse_success(invoke(temp.path(), &["stop", run_id, "--grace", "100ms"]));
    assert_eq!(stopped.run.state.status, RunStatus::Stopped);
}

#[test]
fn readiness_timeout_is_distinct_from_wait_timeout() {
    let temp = TempDir::new().unwrap();
    let marker = temp.path().join("heartbeat.log");
    let marker_text = marker.to_str().unwrap();
    let binary = binary();
    let binary = binary.to_str().unwrap();
    let start: StartResult = parse_success(invoke(
        temp.path(),
        &[
            "start",
            "--ready-log",
            "never-appears",
            "--readiness-timeout",
            "100ms",
            "--",
            binary,
            "__fixture",
            "tree",
            "--marker",
            marker_text,
        ],
    ));
    let run_id = &start.run.state.run_id;
    let output = invoke(
        temp.path(),
        &["wait", run_id, "--for", "ready", "--timeout", "3s"],
    );
    assert_eq!(output.status.code(), Some(1));
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["kind"], "readiness_failed");

    let status: StatusResult = parse_success(invoke(temp.path(), &["status", run_id]));
    assert_eq!(status.run.state.readiness.status, ReadinessStatus::TimedOut);
    assert_eq!(
        status.run.state.readiness.failure_reason.as_deref(),
        Some("readiness_timeout")
    );
    let _: StopResult = parse_success(invoke(temp.path(), &["stop", run_id, "--grace", "100ms"]));
}

#[test]
fn log_readiness_never_claims_evidence_that_was_dropped() {
    let temp = TempDir::new().unwrap();
    let binary = binary();
    let binary = binary.to_str().unwrap();
    let start: StartResult = parse_success(invoke(
        temp.path(),
        &[
            "start",
            "--max-log-bytes",
            "1",
            "--ready-log",
            "fixture",
            "--readiness-timeout",
            "5s",
            "--",
            binary,
            "__fixture",
            "emit",
        ],
    ));
    let run_id = &start.run.state.run_id;
    let ready = invoke(
        temp.path(),
        &["wait", run_id, "--for", "ready", "--timeout", "5s"],
    );
    assert_eq!(ready.status.code(), Some(1));
    let status: StatusResult = parse_success(invoke(temp.path(), &["status", run_id]));
    assert_eq!(status.run.state.readiness.status, ReadinessStatus::Failed);
    assert_eq!(status.run.state.logs.captured_bytes, 1);
    assert_eq!(status.run.state.logs.dropped_bytes, 9);
}

#[test]
fn named_leases_resolve_argv_and_are_released_with_evidence() {
    let temp = TempDir::new().unwrap();
    let binary = binary();
    let binary = binary.to_str().unwrap();
    let start: StartResult = parse_success(invoke(
        temp.path(),
        &[
            "start",
            "--lease-port",
            "web",
            "--ready-port",
            "web",
            "--lease-temp-dir",
            "work",
            "--readiness-timeout",
            "5s",
            "--",
            binary,
            "__fixture",
            "serve",
            "--address",
            "127.0.0.1:{port:web}",
            "--touch",
            "{temp:work}/ready.flag",
        ],
    ));
    let run_id = &start.run.state.run_id;
    let ready: WaitResult = parse_success(invoke(
        temp.path(),
        &["wait", run_id, "--for", "ready", "--timeout", "5s"],
    ));
    assert_eq!(ready.run.state.readiness.status, ReadinessStatus::Ready);
    assert_eq!(
        ready.run.state.command.environment.injected_names,
        ["PROCHERD_PORT_WEB", "PROCHERD_TEMP_WORK"]
    );
    assert!(
        ready
            .run
            .state
            .command
            .args
            .iter()
            .all(|argument| !argument.contains("{port:") && !argument.contains("{temp:"))
    );

    let leases: LeasesResult = parse_success(invoke(temp.path(), &["leases", run_id]));
    assert_eq!(leases.leases.ports.len(), 1);
    assert!(leases.leases.ports[0].port.is_some());
    assert!(leases.leases.ports[0].handoff_gap_ms.is_some());
    assert!(leases.leases.ports[0].released_at_ms.is_none());
    let temp_path = leases.leases.temp_directories[0].path.as_ref().unwrap();
    assert!(temp_path.join("ready.flag").is_file());

    let stopped: StopResult =
        parse_success(invoke(temp.path(), &["stop", run_id, "--grace", "100ms"]));
    assert!(stopped.run.state.leases.ports[0].released_at_ms.is_some());
    assert!(
        stopped.run.state.leases.temp_directories[0]
            .released_at_ms
            .is_some()
    );

    let registry: serde_json::Value =
        serde_json::from_slice(&fs::read(temp.path().join("port-leases.json")).unwrap()).unwrap();
    assert_eq!(registry["entries"].as_array().unwrap().len(), 0);
}

#[test]
fn lease_environment_is_delivered_and_registry_prevents_reuse() {
    let temp = TempDir::new().unwrap();
    let binary = binary();
    let binary = binary.to_str().unwrap();
    let first: StartResult = parse_success(invoke(
        temp.path(),
        &[
            "start",
            "--lease-port",
            "web",
            "--lease-temp-dir",
            "work",
            "--",
            binary,
            "__fixture",
            "write-env",
            "--name",
            "PROCHERD_PORT_WEB",
            "--path",
            "{temp:work}/port.txt",
        ],
    ));
    let first_run_id = &first.run.state.run_id;
    let first_port = first.run.state.leases.ports[0].port.unwrap();
    let first_temp = first.run.state.leases.temp_directories[0]
        .path
        .as_ref()
        .unwrap();
    let port_file = first_temp.join("port.txt");
    if !condition_met_within(Duration::from_secs(5), || port_file.is_file()) {
        let status = invoke(temp.path(), &["status", first_run_id]);
        let logs = invoke(temp.path(), &["logs", first_run_id]);
        let state = fs::read_to_string(temp.path().join(first_run_id).join("state.json"))
            .unwrap_or_else(|error| format!("cannot read state: {error}"));
        panic!(
            "leased child did not create {}\nstatus stdout: {}\nstatus stderr: {}\nlogs stdout: {}\nlogs stderr: {}\nstate: {}",
            port_file.display(),
            String::from_utf8_lossy(&status.stdout),
            String::from_utf8_lossy(&status.stderr),
            String::from_utf8_lossy(&logs.stdout),
            String::from_utf8_lossy(&logs.stderr),
            state,
        );
    }
    assert_eq!(
        fs::read_to_string(&port_file).unwrap(),
        first_port.to_string()
    );

    let marker = temp.path().join("second-heartbeat.log");
    let marker_text = marker.to_str().unwrap();
    let second: StartResult = parse_success(invoke(
        temp.path(),
        &[
            "start",
            "--lease-port",
            "web",
            "--",
            binary,
            "__fixture",
            "tree",
            "--marker",
            marker_text,
        ],
    ));
    let second_port = second.run.state.leases.ports[0].port.unwrap();
    assert_ne!(first_port, second_port);

    let _: StopResult = parse_success(invoke(
        temp.path(),
        &["stop", first_run_id, "--grace", "100ms"],
    ));
    let _: StopResult = parse_success(invoke(
        temp.path(),
        &["stop", &second.run.state.run_id, "--grace", "100ms"],
    ));
}

#[test]
fn garbage_collection_is_dry_run_by_default_and_rechecks_terminal_state() {
    let temp = TempDir::new().unwrap();
    let start = start_fixture(temp.path(), &["emit"]);
    let run_id = &start.run.state.run_id;
    let _: WaitResult = parse_success(invoke(
        temp.path(),
        &["wait", run_id, "--for", "exit", "--timeout", "5s"],
    ));

    let planned: GcResult = parse_success(invoke(temp.path(), &["gc", "--older-than", "0ms"]));
    let plan = planned
        .entries
        .iter()
        .find(|entry| entry.run_id == *run_id)
        .unwrap();
    assert!(plan.eligible);
    assert!(!plan.deleted);
    let _: StatusResult = parse_success(invoke(temp.path(), &["status", run_id]));

    let executed: GcResult = parse_success(invoke(
        temp.path(),
        &["gc", "--older-than", "0ms", "--execute"],
    ));
    let deletion = executed
        .entries
        .iter()
        .find(|entry| entry.run_id == *run_id)
        .unwrap();
    assert!(deletion.deleted);
    let missing = invoke(temp.path(), &["status", run_id]);
    assert_eq!(missing.status.code(), Some(4));
}

#[cfg(unix)]
#[test]
fn supervisor_lock_symlinks_are_rejected_as_integrity_failures() {
    use std::os::unix::fs::symlink;

    let temp = TempDir::new().unwrap();
    let start = start_fixture(temp.path(), &["emit"]);
    let run_id = &start.run.state.run_id;
    let _: WaitResult = parse_success(invoke(
        temp.path(),
        &["wait", run_id, "--for", "exit", "--timeout", "5s"],
    ));

    let lock_path = temp.path().join(run_id).join("supervisor.lock");
    fs::remove_file(&lock_path).unwrap();
    symlink(temp.path().join(run_id).join("state.json"), &lock_path).unwrap();

    let output = invoke(temp.path(), &["status", run_id]);
    assert_eq!(output.status.code(), Some(5));
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["kind"], "integrity");
}

#[test]
fn maximum_runtime_stops_the_tree_with_a_distinct_reason() {
    let temp = TempDir::new().unwrap();
    let marker = temp.path().join("runtime-heartbeat.log");
    let marker_text = marker.to_str().unwrap();
    let binary = binary();
    let binary = binary.to_str().unwrap();
    let start: StartResult = parse_success(invoke(
        temp.path(),
        &[
            "start",
            "--max-runtime",
            if cfg!(windows) { "1s" } else { "150ms" },
            "--runtime-grace",
            "100ms",
            "--",
            binary,
            "__fixture",
            "tree",
            "--marker",
            marker_text,
        ],
    ));
    let run_id = &start.run.state.run_id;
    let finished: WaitResult = parse_success(invoke(
        temp.path(),
        &["wait", run_id, "--for", "exit", "--timeout", "5s"],
    ));
    assert_eq!(finished.run.state.status, RunStatus::Stopped);
    assert_eq!(
        finished.run.state.exit.as_ref().unwrap().reason,
        ExitReason::RuntimeLimit
    );
    assert!(
        finished
            .run
            .state
            .limits
            .runtime_limit_triggered_at_ms
            .is_some()
    );
    let settled_size = fs::metadata(&marker).unwrap().len();
    thread::sleep(Duration::from_millis(300));
    assert_eq!(fs::metadata(&marker).unwrap().len(), settled_size);
}

#[cfg(unix)]
#[test]
fn supervisor_io_failure_kills_the_owned_process_tree() {
    use std::os::unix::fs::PermissionsExt;

    let temp = TempDir::new().unwrap();
    let marker = temp.path().join("heartbeat.log");
    let marker_text = marker.to_str().unwrap();
    let start = start_fixture(temp.path(), &["tree", "--marker", marker_text]);
    let run_id = &start.run.state.run_id;
    wait_until(Duration::from_secs(5), || {
        fs::metadata(&marker).is_ok_and(|metadata| metadata.len() > 0)
    });

    let run_dir = temp.path().join(run_id);
    fs::set_permissions(&run_dir, fs::Permissions::from_mode(0o500)).unwrap();
    wait_until(Duration::from_secs(5), || {
        let output = invoke(temp.path(), &["status", run_id]);
        output.status.success()
            && serde_json::from_slice::<StatusResult>(&output.stdout)
                .is_ok_and(|status| !status.run.supervisor_active)
    });
    fs::set_permissions(&run_dir, fs::Permissions::from_mode(0o700)).unwrap();

    let settled_size = fs::metadata(&marker).unwrap().len();
    thread::sleep(Duration::from_millis(300));
    assert_eq!(fs::metadata(&marker).unwrap().len(), settled_size);
    let status: StatusResult = parse_success(invoke(temp.path(), &["status", run_id]));
    assert!(!status.run.supervisor_active);
    assert!(
        status.run.state.status == RunStatus::Failed
            || status.run.observed_status == procherd::model::ObservedStatus::Orphaned
    );
}

fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) {
    assert!(
        condition_met_within(timeout, &mut condition),
        "condition was not met within {timeout:?}"
    );
}

fn condition_met_within(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if condition() {
            return true;
        }
        thread::sleep(Duration::from_millis(25));
    }
    false
}
