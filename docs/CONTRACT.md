# CLI and data contract

## Compatibility

Every structured top-level result has a `schema_version`. Full JSON Schemas are
available with `procherd schema --document <name>` for `run`, `start`,
`status`, `list`, `logs`, `wait`, `stop`, `leases`, `gc`, and `error`.
Pre-1.0 additions may occur, but existing meanings are changed only with
release notes and a versioned schema.

Run IDs are canonical uppercase ULIDs prefixed with `run_`. Cursors are
monotonic unsigned integers scoped to one run; callers must treat them as
opaque resume positions rather than byte offsets.

Durable v1 run documents and log records use closed shapes. Readers reject
unknown fields, unsupported schema identifiers, cross-run identity, malformed
Base64, byte-count mismatches, non-monotonic cursors, cursors outside the
durable summary, and incomplete terminal records as integrity failures.
Fields explicitly documented with defaults remain readable when omitted. The
versioned fixtures and declared mutations in
[`tests/fixtures/contracts/`](../tests/fixtures/contracts/README.md) exercise
these rules without starting a child process.

## Lifecycle

`start` returns after the child has been spawned or after a terminal startup
outcome is fully settled. Without readiness options, a successful start does
not mean ready.

`wait` conditions are:

- `running`: a child PID has been recorded;
- `ready`: every configured readiness check has evidence;
- `exit`: the lifecycle is terminal and the supervisor lock is inactive.

`stop` is idempotent. A repeated call on a settled terminal run succeeds with
`already_terminal: true`.

## Readiness

All supplied checks must pass. TCP and HTTP destinations must be loopback.
HTTP status codes 200 through 399 pass. File checks require a regular file.
Log literals are exact UTF-8 byte sequences and can span capture chunks, but
only retained bytes count as evidence.

A readiness deadline belongs to the run. A caller's `wait --timeout` only
limits that caller and does not mutate the readiness deadline.

## Logs

Log records include cursor, millisecond timestamp, stream, base64 encoding,
data, and byte count. `next_after_cursor` is the cursor to pass on the next
request. `has_more` says that more retained records currently exist.

`captured_bytes` and `dropped_bytes` are cumulative. `terminal` describes the
durable run state. A terminal result can still be read after the supervisor
has exited.

## Errors and exit codes

With JSON or NDJSON formatting, errors are emitted on stderr as
`procherd.error.v1`:

```json
{
  "schema_version": "procherd.error.v1",
  "error": {
    "kind": "not_found",
    "message": "run not found: run_...",
    "exit_code": 4
  }
}
```

Exit-code classes are stable:

- `0`: success;
- `1`: operational failure;
- `2`: usage error;
- `3`: timeout;
- `4`: run not found;
- `5`: durable-state integrity failure.

Callers should branch on the numeric class and structured `kind`, not parse
human text.

## Storage mutation

Status, list, logs, leases, wait, schema, and completions are read-only.
`start` creates one run. `stop` creates an authenticated stop request. `gc` is
read-only unless `--execute` is present; execution rechecks every candidate
immediately before deletion.
