# ProcHerd concept

## Thesis

ProcHerd gives software agents durable ownership of local processes, bounded
logs, explicit readiness, and leased resources without requiring a terminal
session or shared service to remain alive.

## Primary job

Start one bounded local process from an argument vector, receive a stable run
ID, wait for declared evidence, and later inspect or stop its complete owned
process tree.

The target users are coding agents, local orchestration frameworks, developer
tools, and humans operating agent-created jobs.

## Design decisions

### Stable identity, live ownership

A run ID is a canonical `run_<ULID>`, never a PID. Each run has a detached
supervisor and an operating-system lock. Durable state can therefore be read
offline while liveness is determined from the lock rather than PID reuse.

ProcHerd deliberately uses one small supervisor per run instead of a shared
broker. This removes daemon installation, upgrade coordination, and a single
failure domain. The tradeoff is one additional process per active run.

### Argument vectors, not shell reconstruction

The public start contract is a UTF-8 program plus arguments. Shell behavior is
available only when the caller explicitly names a shell as the program.
ProcHerd never parses, interpolates, quotes, or reconstructs a shell string.

### Bounded evidence

Stdout and stderr are drained concurrently into ordered, cursor-addressed
records. Retention has a hard byte cap. Bytes beyond the cap are still drained
and hashed but reported as dropped, so output pressure cannot turn into a
hidden process deadlock.

### Readiness is evidence

Spawn success and application readiness are distinct. Configured readiness
conditions are AND-composed, retain per-check evidence and timestamps, and
fail if the process exits first or the readiness deadline expires.

### Leases expose their guarantee

Temporary directories are private paths owned by the run. Loopback ports are
reserved and coordinated across active ProcHerd runs. The handoff to a child
that binds its own port necessarily contains a small race, so the durable
contract records the handoff gap and names the guarantee
`coordinated_best_effort`.

### Cleanup is observable

Unix children start in a new process group; Windows children are assigned to a
Job Object. Stop and runtime-limit paths record requested time, grace period,
forced termination, descendant cleanup, and completion. Repeated stop calls
are safe.

## Lifecycle

```text
created -> starting -> running -> exited
                     \-> failed
running -> stopping -> stopped
running -> stopping -> stopped (runtime_limit)
```

Readiness is an orthogonal state:

```text
not_configured
pending -> ready
pending -> timed_out
pending -> failed (process exited first)
```

`observed_status: orphaned` is a derived observation when durable state claims
a live lifecycle state but the supervisor lock is inactive. Version 0.1
reports this condition and refuses unsafe mutation; it does not promise crash
reattachment.

## Initial scope

Version 0.1 supports one-machine, one-user execution on Linux, macOS, and
Windows:

- detached start, status, list, wait, logs, stop, leases, and conservative GC;
- Unix process-group and Windows Job Object cleanup;
- local TCP, local HTTP, regular-file, retained-log-literal, and leased-port
  readiness;
- named loopback ports and private temporary directories;
- bounded structured logs with complete-stream SHA-256;
- maximum runtime and bounded shutdown grace;
- human, versioned JSON/NDJSON, JSON Schema, and completion output.

## Explicit non-goals for 0.1

- distributed scheduling, containers, remote execution, or multi-tenancy;
- a boot-time service manager or automatic restart policy;
- PTY, interactive stdin, or implicit shell execution;
- HTTPS, remote, authenticated, or custom-header readiness probes;
- CPU, memory, or process-count isolation;
- encrypted or redacted log storage;
- atomic port transfer to arbitrary child programs;
- recovery or reattachment after supervisor or machine failure;
- containment of programs that intentionally escape OS process-tree controls.

## Success measures

- zero leaked descendants in repeated lifecycle fixtures;
- no readiness claim without retained, inspectable evidence;
- deterministic reacquisition and cursor pagination after the launching client
  exits;
- explicit rather than silent log loss and port-handoff gaps;
- cross-platform release checks, SBOMs, provenance, and verified archives;
- adoption as a subprocess backend in at least three opt-in external
  workflows before 1.0.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the implementation model
and [ROADMAP.md](ROADMAP.md) for the compatibility path to 1.0.
