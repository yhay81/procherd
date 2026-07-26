# ProcHerd concept

## One-line thesis

ProcHerd gives software agents durable ownership of local processes, logs,
readiness, and leased resources without requiring a terminal session to remain
alive.

## Problem

Starting a development server is easy for a human with a terminal. It is
surprisingly difficult for an agent that may be interrupted, resumed, or
replaced:

- process IDs are reused and child processes escape;
- output floods context or disappears with the shell;
- ports and temporary directories collide;
- "started" does not mean "ready";
- cancellation leaves processes behind;
- another agent cannot safely reacquire ownership.

Shell backgrounding and terminal multiplexers expose implementation details
instead of a compact lifecycle contract.

## Target users and jobs

- Coding agents running test servers, watchers, and long builds.
- Local agent orchestration frameworks.
- Developer tools that need resilient subprocess ownership.
- Humans who want inspectable agent-created local jobs.

The primary job is: **start a bounded local process, receive a stable run ID,
wait for a declared condition, and later inspect or stop the entire process
tree.**

## Product principles

1. Stable run identity is separate from operating-system PID.
2. Process-tree ownership is explicit.
3. Readiness is a declared condition with evidence.
4. Logs are cursor-based and bounded.
5. Ports, paths, and other exclusive resources are leases.
6. Cleanup is observable and idempotent.
7. Local, same-user operation works with zero manual daemon setup.

## Proposed command contract

```text
procherd schema --brief --format json
procherd start --spec run.json --format json
procherd status <run-id> --format json
procherd wait <run-id> --for ready --timeout 30s --format json
procherd logs <run-id> --after <cursor> --limit 200 --format ndjson
procherd stop <run-id> --grace 5s --format json
procherd leases <run-id> --format json
procherd gc --dry-run --format json
```

The process argument vector is an array in the spec, never a shell string unless
the caller explicitly requests a shell interpreter.

## Run specification

A run spec can declare:

- argument vector, working directory, and environment references;
- stdin behavior and terminal requirements;
- startup, runtime, idle, and shutdown deadlines;
- restart policy and maximum attempts;
- readiness checks: port, HTTP, file, log pattern, or child exit;
- leased ports and temporary directories;
- log retention and redaction rules;
- CPU, memory, and process-count limits where supported;
- cleanup hooks with explicit time budgets.

## Lifecycle model

```text
created -> starting -> ready/running -> exited
                    \-> failed
ready/running -> stopping -> stopped
any live state -> orphaned -> recovered/stopped
```

Every transition has a timestamp, reason code, and evidence. A run that exits
before readiness is not reported as successfully started.

## Ownership and persistence

A small same-user broker maintains run state under the platform's standard data
directory. The CLI starts or reconnects to it automatically. It records process
groups or platform job objects, leases, log segments, and lifecycle events.

Reacquisition uses the stable run ID and a scoped owner token. Status remains
readable without the token; mutation requires ownership or an explicit
administrative operation.

## Log contract

Logs are stored outside agent context and read through monotonic cursors.
Callers can request:

- stdout, stderr, or both;
- records after a cursor;
- line and byte limits;
- time windows;
- matching records plus bounded context;
- a digest for the complete retained stream.

Dropped or expired log data is reported explicitly.

## Initial scope

Version 0.1 will support:

- macOS and Linux local processes;
- process-tree start, wait, inspect, stop, and cleanup;
- port and temporary-directory leases;
- port, HTTP, file, and log-pattern readiness;
- cursor-based structured logs;
- automatic same-user broker startup;
- crash recovery and conservative garbage collection.

## Non-goals

- A distributed job scheduler.
- Container orchestration.
- Multi-tenant security or remote execution.
- A general service manager for operating-system boot.
- Hiding whether a command requested a shell.
- Inferring application readiness from elapsed time alone.

## Differentiation and defensibility

ProcHerd focuses on the lifecycle gap between a subprocess library and a
distributed scheduler. Its agent-native contract combines durable IDs, ownership,
readiness, leased resources, and bounded logs. Cross-platform process correctness
and integrations with agent frameworks can become a meaningful moat.

## Success measures

- Zero leaked descendant processes in the lifecycle fixture suite.
- Port-collision and stale-lease rates.
- Successful reacquisition after client interruption.
- Readiness false-positive rate.
- Median log bytes and tokens retrieved per debugging task.
- Adoption as a subprocess backend in agent frameworks.

## Key risks and open questions

- Process-tree control differs substantially across platforms.
- Daemon crashes can desynchronize recorded and actual state.
- Owner-token ergonomics must not undermine local security.
- Some commands daemonize or deliberately escape process groups.
- Resource limiting without containers is platform-dependent.

ProcHerd must expose capability differences rather than pretending every
platform can enforce the same guarantees.
