# RunCradle

Durable local process control for software agents.

> Status: concept stage. This repository currently contains no process supervisor.

RunCradle gives one-off commands durable IDs, bounded logs, readiness checks, and leased resources. A process remains discoverable after the shell or agent turn that launched it has ended.

```bash
runcradle start --lease-port 1 -- npm run dev
runcradle status run_01J...
runcradle wait run_01J... --http-ready /health --timeout 60s
runcradle logs run_01J... --since cursor_... --limit 50
runcradle stop run_01J...
```

## Why

Agents routinely lose background PIDs, consume unbounded logs, race for ports, and leave process trees behind. General-purpose supervisors are optimized for declared long-lived services, not dynamic one-off agent jobs.

## Product principles

- Stable run IDs instead of shell-local PIDs.
- Argument arrays, never shell-string reconstruction.
- Bounded and cursor-based logs.
- Explicit port and temporary-directory leases.
- Readiness and health are structured conditions.
- Process-tree cleanup produces a receipt.
- Zero project configuration for the common path.

## Initial scope

One-machine, one-user local execution on macOS, Linux, and Windows. Distributed scheduling and container orchestration are out of scope.

See [CONCEPT.md](CONCEPT.md) for lifecycle and resource semantics.

## License

MIT
