# ProcHerd

Durable local process control for software agents.

> Status: 0.2 release. The complete local lifecycle is implemented
> and tested on Linux, macOS, and Windows.

ProcHerd starts a command under a detached per-run supervisor and returns a
stable run ID. A later shell, agent turn, or local tool can inspect state, wait
for readiness, page through bounded logs, or stop the entire owned process
tree.

```bash
procherd start \
  --lease-port web \
  --ready-port web \
  -- npm run dev -- --port {port:web}

procherd wait run_01J... --for ready --timeout 60s
procherd logs run_01J... --after 0 --limit 50
procherd stop run_01J... --grace 5s
```

## Why

Shell backgrounding gives callers a PID, but not durable ownership. Agent
workflows routinely lose background processes, fill pipes or context with
output, race for local ports, confuse “spawned” with “ready,” and leave
descendants behind.

ProcHerd makes those lifecycle boundaries explicit:

- stable `run_<ULID>` identity independent of process IDs;
- a detached supervisor per run, with no shared daemon to configure;
- Unix process groups or Windows Job Objects for tree cleanup;
- bounded stdout/stderr capture with monotonic cursors and full-stream digests;
- AND-composed TCP, HTTP, file, log-literal, and leased-port readiness;
- named loopback-port and private temporary-directory leases;
- maximum runtime, graceful shutdown, forced cleanup, and durable evidence;
- versioned JSON results, JSON Schemas, stable exit codes, and completions.

## Install

Download a native archive from
[GitHub Releases](https://github.com/yhay81/procherd/releases), or install from
a source checkout with Rust 1.85 or newer:

```bash
cargo install --path . --locked
```

See [INSTALL.md](INSTALL.md) for platform-specific, checksum- and
provenance-verified native installation, updating, and removal.

Generate completion scripts with `procherd completions bash` (also `zsh`,
`fish`, `powershell`, and `elvish`).

## Start and reacquire a run

Commands are always passed as an argument vector after `--`; ProcHerd never
reconstructs a shell string.

```bash
result=$(
  procherd --format json start \
    --max-runtime 10m \
    --ready-log "listening" \
    -- ./my-server --port 8080
)

run_id=$(printf '%s' "$result" | jq -r .run.run_id)
procherd --format json status "$run_id"
procherd --format json wait "$run_id" --for ready --timeout 30s
```

When no readiness condition is supplied, `start` means the child was spawned;
it does not claim application readiness. `wait --for exit` returns only after
the terminal state is durable and the per-run supervisor has fully exited.

## Leased resources

Named resources are delivered both through placeholders and environment
variables:

```bash
procherd start \
  --lease-port api \
  --lease-temp-dir work \
  --ready-port api \
  -- ./server \
       --port {port:api} \
       --work-dir {temp:work}
```

The child also receives `PROCHERD_PORT_API` and `PROCHERD_TEMP_WORK`. Names use
lowercase ASCII identifiers and are normalized to uppercase (with `-` changed
to `_`) in environment variables.
Temporary directories remain available for inspection until the run is
garbage-collected.

Port allocation is coordinated among ProcHerd runs and the listener is held
until immediately before spawn. It is intentionally reported as
`coordinated_best_effort`: operating systems do not provide an atomic transfer
of a listening socket to an arbitrary child that binds the port itself.

## Bounded logs

Logs are captured in separate stdout/stderr records using base64 so arbitrary
bytes remain lossless:

```bash
procherd --format json logs run_01J... --after 0 --limit 200
procherd --format ndjson logs run_01J... --stream stderr
```

Use `next_after_cursor` for the next page. Capture stops at
`--max-log-bytes` (16 MiB by default); the supervisor continues draining child
pipes through a fixed-capacity in-memory queue and reports `dropped_bytes`.
Final SHA-256 digests cover the complete stdout and stderr streams, including
bytes not retained as records. If output arrives faster than it can be hashed,
bounded pipe backpressure can temporarily throttle the child instead of
allowing supervisor memory to grow with the stream.

## Machine contract

All commands accept `--format human|json|ndjson`. Machine-readable results use
versioned schema identifiers. Inspect the bounded contract or a full JSON
Schema without starting a process:

```bash
procherd --format json schema --document brief
procherd --format json schema --document run
procherd --format json schema --document start
procherd --format json schema --document logs
```

Stable exit-code classes are:

| Code | Meaning |
| ---: | --- |
| 0 | Success |
| 1 | Operational failure |
| 2 | Invalid usage |
| 3 | Caller or readiness timeout |
| 4 | Run not found |
| 5 | Durable-state integrity failure |

See [docs/CONTRACT.md](docs/CONTRACT.md) for lifecycle and compatibility
details.

The checked-in
[store compatibility corpus](tests/fixtures/contracts/README.md) freezes a
published v0.1 terminal run store byte-for-byte. CI reopens it through every
read-only lifecycle view and rejects declared state and log corruptions on all
supported operating systems.

Performance observations use a generated 1,000-run terminal store and a
minimal start-to-running lifecycle. The
[benchmark methodology](benchmarks/README.md) documents measurement
boundaries, supervisor memory sampling, raw artifacts, and the distinction
between the current baseline and future v1.0 regression thresholds.

## State and cleanup

The default state root is the platform user data directory:

- Linux: `$XDG_DATA_HOME/procherd` or `~/.local/share/procherd`;
- macOS: `~/Library/Application Support/procherd`;
- Windows: `%LOCALAPPDATA%\procherd`.

Override it with `--state-dir` or `PROCHERD_STATE_DIR`. On Unix, new state
directories and files use owner-only permissions. `gc` is a dry run unless
`--execute` is supplied and rechecks that every target is terminal and
inactive immediately before deletion:

```bash
procherd --format json gc --older-than 7d
procherd --format json gc --older-than 7d --execute
```

## Security boundaries

- ProcHerd executes the exact program requested with the caller's inherited
  environment. It is not a sandbox and does not restrict filesystem or network
  access.
- Captured logs and command arguments may contain secrets. Logs are bounded,
  but not redacted or encrypted.
- Status is readable by any principal that can read the state directory.
  Stop requests require the private per-run owner token stored there.
- Readiness is local-only: TCP and HTTP endpoints must resolve to loopback;
  HTTP 2xx and 3xx responses count as ready. HTTPS and credential-bearing URLs
  are rejected in 0.1.
- Commands that deliberately daemonize or escape their assigned process group
  may exceed Unix cleanup guarantees. Windows descendants remain in the Job
  Object unless they use operating-system escape privileges.
- A machine crash can leave a live-looking durable record. ProcHerd reports it
  as `orphaned`; automatic process reattachment is not a 0.1 guarantee.

Read [SECURITY.md](SECURITY.md) and
[docs/PLATFORM-SUPPORT.md](docs/PLATFORM-SUPPORT.md) before relying on ProcHerd
for sensitive or long-lived workloads.

## Development

```bash
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --all-targets --locked
cargo package --locked --allow-dirty
```

The E2E suite covers detached reacquisition, cursor pagination, log overflow,
readiness success and false-positive prevention, lease handoff and collision
avoidance, runtime limits, idempotent tree cleanup, supervisor failure, and
safe garbage collection.

## Release integrity

CI tests Linux, macOS, Windows, and Rust 1.85. Tagged releases contain native
archives, documentation, completions, SHA-256 checksums, a CycloneDX SBOM, and
GitHub/Sigstore build-provenance and SBOM attestations. See
[RELEASING.md](RELEASING.md).

## Community

Use [GitHub Discussions](https://github.com/yhay81/procherd/discussions) for
questions and workflow examples, and structured issues for reproducible bugs
and scoped features. See [CONTRIBUTING.md](CONTRIBUTING.md),
[SUPPORT.md](SUPPORT.md), [GOVERNANCE.md](GOVERNANCE.md), and the
[Code of Conduct](CODE_OF_CONDUCT.md). Report vulnerabilities privately.

Verified, opt-in usage is recorded in [ADOPTERS.md](ADOPTERS.md).

## License

MIT
