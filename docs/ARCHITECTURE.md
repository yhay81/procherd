# Architecture

## Components

The public `procherd` binary has two roles:

1. the foreground CLI validates input, creates immutable run identity and
   initial state, launches a detached supervisor, and returns;
2. the hidden supervisor role owns the child process tree, drains output,
   evaluates readiness and deadlines, persists state, and releases resources.

There is no shared daemon. Each active run is isolated in its own supervisor
process and state directory.

## Run directory

```text
<state-root>/
  port-leases.lock
  port-leases.json
  run_<ULID>/
    supervisor.lock
    owner.token
    spec.json
    state.json
    stop.request.json       # transient, only while stopping
    logs.ndjson
    resources/temp/<name>/  # when requested
```

JSON state writes are atomic replacements. Input documents are bounded and
run/state paths that resolve to symlinks are rejected. On Unix, created
directories use mode `0700` and files use `0600`.

The operating-system lock is the liveness authority. PIDs are retained for
diagnostics and process control, never used alone to infer ownership.

## Process ownership

On Unix, the detached supervisor creates a new session and the child creates a
new process group. Stop sends the graceful signal to the group, waits for the
requested grace period, then force-kills the group if needed.

On Windows, the detached supervisor assigns the child to a Job Object whose
close/termination semantics cover descendants. Windows 0.1 cleanup is forced;
there is no portable graceful console signal guarantee. The supervisor is
created with Win32 handle inheritance disabled so that a long-lived run cannot
retain stdout or stderr capture pipes owned by the foreground caller. This
small Win32 FFI boundary is the only audited exception to the crate-wide
`unsafe_code` deny lint.

Supervisor-side ownership is RAII-backed: unexpected supervisor I/O failures
trigger best-effort tree termination before the supervisor exits.

## State machine and observation

Lifecycle state is durable; `supervisor_active` is observed from the lock.
`observed_status` is `orphaned` only when durable state claims a live state but
the supervisor is inactive. Because the supervisor can finalize between a
state read and a lock check, an inactive observation is paired with a second
state read before declaring a run orphaned.

Terminal API boundaries include supervisor shutdown:

- `wait --for exit` requires terminal durable state and an inactive lock;
- `stop` waits for the same boundary, including repeated calls;
- `start` waits for supervisor shutdown before returning a terminal or failed
  result for very short-lived children.

This prevents a caller from treating a recorded exit as completed cleanup.

## Output flow

Stdout and stderr readers send 8 KiB chunks through a fixed-capacity queue to
the supervisor loop. Each control-loop pass processes a fixed maximum batch so
stop requests, runtime limits, readiness, and process exit cannot be starved by
continuous output. Records share one monotonic cursor and preserve stream
identity and arbitrary bytes through base64. The retention budget is global
across both streams.

After the budget is exhausted, the readers continue draining and hashing.
Durable summaries distinguish `captured_bytes` and `dropped_bytes`; final
stream digests cover both. Log-based readiness only observes retained bytes,
so ProcHerd never claims evidence that callers cannot retrieve.

The queue applies bounded backpressure when a child produces output faster than
the supervisor can hash it. After process exit, terminal state is persisted only
after both readers close; a bounded drain timeout becomes an explicit
`log_drain_timeout` supervisor failure rather than a successful partial digest.

Records are flushed before the periodic durable state snapshot. Live readers
therefore stop at the snapshot's committed cursor boundary and retry on their
next poll; they do not misclassify a fully written but not-yet-committed record
as corruption. Terminal readers still require every record to fall within the
final durable summary.

## Resource coordination

Port allocation uses a state-root registry protected by an OS file lock.
Stale registry entries are pruned by checking the corresponding supervisor
lock. ProcHerd holds a loopback listener during setup, releases it immediately
before child spawn, and records the handoff timestamps.

Temporary resources live under the run directory. Lease registry entries are
released at terminal cleanup; temporary files remain until GC for inspection.

## Failure model

Setup, lease, spawn, readiness, runtime, stop, and supervisor failures have
durable kind/reason fields. An inability to launch the supervisor is recorded
directly by the foreground CLI. Once the supervisor owns the run, it is
responsible for terminal state and resource release.

Machine power loss is outside the transactional boundary. On the next read,
state may appear orphaned. Version 0.1 reports that discrepancy and avoids
unsafe assumptions rather than attempting PID-based reattachment.
