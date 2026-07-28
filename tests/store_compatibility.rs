use std::collections::BTreeSet;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use procherd::model::{
    GcResult, LeasesResult, ListResult, LogRecord, LogsResult, RunState, StatusResult, StopResult,
    WaitResult,
};
use procherd::store::Store;
use serde_json::Value;
use sha2::{Digest, Sha256};
use tempfile::TempDir;

const RUN_ID: &str = "run_01J00000000000000000000000";

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_procherd"))
}

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/contracts/v0.1")
}

fn fixture_run_root() -> PathBuf {
    corpus_root().join(RUN_ID)
}

fn read_json(path: &Path) -> Value {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn encode_lower(bytes: impl AsRef<[u8]>) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.as_ref().len().saturating_mul(2));
    for byte in bytes.as_ref() {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn materialize_store() -> TempDir {
    let store = tempfile::tempdir().expect("temporary store");
    let run_dir = store.path().join(RUN_ID);
    fs::create_dir(&run_dir).expect("fixture run directory");
    for name in ["state.json", "logs.ndjson", "owner.token"] {
        fs::copy(fixture_run_root().join(name), run_dir.join(name))
            .unwrap_or_else(|error| panic!("copy fixture {name}: {error}"));
    }
    File::create(run_dir.join("supervisor.lock")).expect("fixture supervisor lock");
    store
}

fn invoke(state_dir: &Path, arguments: &[&str]) -> Output {
    Command::new(binary())
        .arg("--state-dir")
        .arg(state_dir)
        .arg("--format")
        .arg("json")
        .args(arguments)
        .output()
        .expect("run procherd")
}

fn parse_success<T: serde::de::DeserializeOwned>(output: Output) -> T {
    assert!(
        output.status.success(),
        "command failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("valid JSON response")
}

fn apply_json_mutation(document: &mut Value, operation: &str, pointer: &str, value: Value) {
    match operation {
        "replace" => {
            let target = document
                .pointer_mut(pointer)
                .unwrap_or_else(|| panic!("replace target {pointer} exists"));
            *target = value;
        }
        "add" => {
            let (parent_pointer, encoded_key) = pointer
                .rsplit_once('/')
                .unwrap_or_else(|| panic!("object pointer {pointer} has a parent"));
            let parent = if parent_pointer.is_empty() {
                document
            } else {
                document
                    .pointer_mut(parent_pointer)
                    .unwrap_or_else(|| panic!("object parent {parent_pointer} exists"))
            };
            let key = encoded_key.replace("~1", "/").replace("~0", "~");
            assert!(
                parent
                    .as_object_mut()
                    .expect("mutation parent is an object")
                    .insert(key, value)
                    .is_none(),
                "add target {pointer} must not exist"
            );
        }
        other => panic!("unsupported JSON mutation {other}"),
    }
}

fn assert_declared_objects_are_closed(value: &Value, path: &str) {
    match value {
        Value::Object(object) => {
            if object.contains_key("properties") {
                assert!(
                    object.get("additionalProperties") == Some(&Value::Bool(false))
                        || object.get("unevaluatedProperties") == Some(&Value::Bool(false)),
                    "object schema at {path} must reject unknown fields"
                );
            }
            for (key, child) in object {
                assert_declared_objects_are_closed(child, &format!("{path}/{key}"));
            }
        }
        Value::Array(values) => {
            for (index, child) in values.iter().enumerate() {
                assert_declared_objects_are_closed(child, &format!("{path}/{index}"));
            }
        }
        _ => {}
    }
}

#[test]
fn current_binary_reopens_the_v01_golden_store() {
    let root = corpus_root();
    let manifest = read_json(&root.join("manifest.json"));
    assert_eq!(manifest["schema_version"], "procherd.store-corpus/v1");
    assert_eq!(manifest["store_version"], "0.1");
    assert_eq!(manifest["run_id"], RUN_ID);

    let mut declared_paths = BTreeSet::new();
    for entry in manifest["files"].as_array().expect("manifest files") {
        let relative_path = entry["path"].as_str().expect("fixture path");
        assert!(
            declared_paths.insert(relative_path.to_owned()),
            "duplicate fixture path {relative_path}"
        );
        let bytes = fs::read(root.join(relative_path)).expect("read fixture");
        assert_eq!(
            encode_lower(Sha256::digest(&bytes)),
            entry["sha256"].as_str().expect("fixture SHA-256"),
            "{relative_path} digest changed"
        );
    }
    let discovered_paths = fs::read_dir(fixture_run_root())
        .expect("read fixture run")
        .map(|entry| entry.expect("read fixture entry").path())
        .filter(|path| path.is_file())
        .map(|path| {
            format!(
                "{RUN_ID}/{}",
                path.file_name()
                    .expect("fixture file name")
                    .to_string_lossy()
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(discovered_paths, declared_paths);

    let state_bytes = fs::read(fixture_run_root().join("state.json")).expect("state fixture");
    let state: RunState = serde_json::from_slice(&state_bytes).expect("deserialize state fixture");
    assert_eq!(
        format!(
            "{}\n",
            serde_json::to_string_pretty(&state).expect("serialize state fixture")
        )
        .as_bytes(),
        state_bytes,
        "state fixture must remain the exact published serialization"
    );
    let log_bytes = fs::read(fixture_run_root().join("logs.ndjson")).expect("log fixture");
    let record: LogRecord = serde_json::from_slice(&log_bytes).expect("deserialize log fixture");
    assert_eq!(
        format!(
            "{}\n",
            serde_json::to_string(&record).expect("serialize log fixture")
        )
        .as_bytes(),
        log_bytes,
        "log fixture must remain the exact published serialization"
    );

    let store = materialize_store();
    let status: StatusResult = parse_success(invoke(store.path(), &["status", RUN_ID]));
    assert_eq!(status.run.state.run_id, RUN_ID);
    assert!(!status.run.supervisor_active);

    let logs: LogsResult = parse_success(invoke(store.path(), &["logs", RUN_ID]));
    assert_eq!(logs.records.len(), 1);
    assert_eq!(logs.records[0].byte_count, 6);
    assert!(logs.terminal);

    let wait: WaitResult = parse_success(invoke(
        store.path(),
        &["wait", RUN_ID, "--for", "exit", "--timeout", "1s"],
    ));
    assert_eq!(wait.run.state.run_id, RUN_ID);
    let stop: StopResult = parse_success(invoke(store.path(), &["stop", RUN_ID]));
    assert!(stop.already_terminal);
    let leases: LeasesResult = parse_success(invoke(store.path(), &["leases", RUN_ID]));
    assert!(leases.leases.ports.is_empty());
    let list: ListResult = parse_success(invoke(store.path(), &["list"]));
    assert_eq!(list.runs.len(), 1);
    let gc: GcResult = parse_success(invoke(store.path(), &["gc", "--older-than", "0ms"]));
    assert!(gc.entries[0].eligible);
    assert!(!gc.entries[0].deleted);

    let typed_store =
        Store::resolve(Some(store.path().to_path_buf())).expect("open typed fixture store");
    assert_eq!(
        typed_store.read_owner_token(RUN_ID).expect("owner token"),
        "a".repeat(64)
    );

    let run_schema: Value = parse_success(invoke(store.path(), &["schema", "--document", "run"]));
    assert_declared_objects_are_closed(&run_schema, "#");
}

#[test]
fn declared_v01_store_corruptions_fail_closed() {
    let manifest = read_json(&corpus_root().join("manifest.json"));
    let mut rejection_ids = BTreeSet::new();

    for case in manifest["rejections"].as_array().expect("rejection cases") {
        let id = case["id"].as_str().expect("rejection ID");
        assert!(
            rejection_ids.insert(id.to_owned()),
            "duplicate rejection ID {id}"
        );
        let store = materialize_store();
        let file_name = case["file"].as_str().expect("mutation file");
        let path = store.path().join(RUN_ID).join(file_name);
        let operation = case["operation"].as_str().expect("mutation operation");

        if operation == "remove_trailing_newline" {
            let mut bytes = fs::read(&path).expect("read mutation target");
            assert_eq!(bytes.pop(), Some(b'\n'));
            fs::write(&path, bytes).expect("write incomplete log mutation");
        } else if operation == "append_duplicate_record" {
            let mut bytes = fs::read(&path).expect("read log mutation target");
            let duplicate = bytes.clone();
            bytes.extend_from_slice(&duplicate);
            fs::write(&path, bytes).expect("write duplicate log mutation");
        } else {
            let mut document = read_json(&path);
            apply_json_mutation(
                &mut document,
                operation,
                case["pointer"].as_str().expect("mutation pointer"),
                case["value"].clone(),
            );
            let serialized = if file_name.ends_with(".ndjson") {
                format!(
                    "{}\n",
                    serde_json::to_string(&document).expect("serialize NDJSON mutation")
                )
            } else {
                format!(
                    "{}\n",
                    serde_json::to_string_pretty(&document).expect("serialize JSON mutation")
                )
            };
            fs::write(&path, serialized).expect("write JSON mutation");
        }

        let output = match case["command"].as_str().expect("mutation command") {
            "status" => invoke(store.path(), &["status", RUN_ID]),
            "logs" => invoke(store.path(), &["logs", RUN_ID]),
            other => panic!("unsupported mutation command {other}"),
        };
        assert_eq!(
            output.status.code(),
            Some(5),
            "rejection {id} did not use the integrity exit class\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let error: Value = serde_json::from_slice(&output.stderr)
            .unwrap_or_else(|parse_error| panic!("parse rejection {id}: {parse_error}"));
        assert_eq!(
            error["error"]["kind"], case["expected_kind"],
            "rejection {id}"
        );
    }
    assert_eq!(rejection_ids.len(), 12);
}
