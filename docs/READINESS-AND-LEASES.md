# Readiness and leases

## Readiness composition

Repeatable readiness flags are AND-composed:

```bash
procherd start \
  --ready-tcp 127.0.0.1:8080 \
  --ready-http http://localhost:8080/health \
  --ready-file ./boot-complete \
  --ready-log "accepting connections" \
  -- ./server
```

Relative file paths are resolved against the child working directory. Local
hostnames must resolve only to loopback addresses. Credential-bearing URLs,
HTTPS, remote addresses, and redirects to remote destinations are not probed.
An HTTP 3xx response itself counts as success; ProcHerd does not follow it.

Each successful check stores a timestamp and evidence string. Pending
readiness becomes `failed` if the child exits first and `timed_out` at the
run's readiness deadline.

## Named ports

Request, inject, and optionally wait for a port:

```bash
procherd start \
  --lease-port web \
  --ready-port web \
  -- ./server --listen 127.0.0.1:{port:web}
```

The resolved port is also available as `PROCHERD_PORT_WEB`. The port registry
prevents two active ProcHerd runs from receiving the same port. The listener is
held until immediately before spawn, after which the child must bind it.

Because this handoff is not atomic, unrelated programs can race for the port.
State exposes `handoff_at_ms`, `handoff_gap_ms`, and
`guarantee: coordinated_best_effort`. Applications should fail clearly if
binding loses the race; callers may then start a new run.

## Named temporary directories

```bash
procherd start \
  --lease-temp-dir work \
  -- ./worker --output {temp:work}
```

The resolved path is also available as `PROCHERD_TEMP_WORK`. ProcHerd creates
the directory privately under the run and releases lifecycle ownership when
the run ends. The directory is retained for inspection until GC removes the
entire run.

## Naming and limits

Lease names contain 1–32 lowercase ASCII letters, digits, underscores, or
hyphens, begin with a lowercase letter, and are unique within their resource
kind. A run may request at most 16 ports, 16 temporary directories, and 16
total readiness conditions.

Port placeholders use `{port:NAME}` and temporary-directory placeholders use
`{temp:NAME}`. An unknown or malformed placeholder is a usage error rather
than being passed through silently.
