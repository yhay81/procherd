# ProcHerd roadmap

ProcHerd evolves in compatibility-preserving lifecycle slices. Every slice
must keep process ownership, output bounds, readiness evidence, lease
guarantees, and machine contracts explicit.

## 0.1 foundation

- [x] Detached per-run supervisors and stable run IDs.
- [x] Unix process-group and Windows Job Object cleanup.
- [x] Durable status, wait, list, stop, leases, and conservative GC.
- [x] Bounded cursor logs, explicit loss, and complete-stream digests.
- [x] Local TCP/HTTP, file, log-literal, and leased-port readiness.
- [x] Coordinated ports and private temporary-directory leases.
- [x] Maximum runtime and observable idempotent cleanup.
- [x] Versioned JSON, JSON Schema, stable exit codes, and completions.
- [x] Linux, macOS, Windows, MSRV, audit, package, and repeated E2E gates.

## 0.2 recovery and compatibility

- Versioned store migration tooling and corruption diagnostics.
- Explicit recovery policy for supervisor or machine interruption.
- Compatibility fixtures for every published JSON and store schema.
- Stress tests for concurrent starts, leases, stop requests, and GC.
- [x] Deterministic 1,000-run store and start-to-running harness with weekly
  raw latency, output-size, controller-memory, and supervisor-memory artifacts.
- Calibrated p95 thresholds plus cleanup, log-pressure, and readiness
  benchmarks.

Current evidence: v0.2 and v0.3 provide two released compatibility cycles. The
current v0.3 reader reopens the digest-pinned v0.1 golden store through status,
logs, wait, stop, list, leases, and dry-run GC. Twelve declared state and log
mutations exercise unknown fields, schema and run identity, Base64 and byte
counts, cursor bounds, and incomplete terminal records. The v0.2 and v0.3
release notes record contract preservation; no migration is required.
Transient-store fixtures are still required.

## 0.3 integrations and policy

- Library-facing orchestration adapter with cancellation-safe ownership.
- Opt-in environment allow/deny policy without recording values.
- Config-file/spec input after a security and compatibility design review.
- Configurable log retention strategies that preserve explicit loss.
- More readiness request controls without allowing remote probes by default.

## v1.0 quality criteria

ProcHerd reaches v1.0 only when every gate below has published, reproducible
evidence. More readiness types, starts, downloads, or stars do not substitute
for process ownership, lease exclusivity, bounded logs, or real use.

### Product and compatibility

- Command, JSON, schema, exit-code, run-store, log-record, stop-request, lease,
  readiness, and owner-token contracts remain compatible across at least two
  released pre-1.0 minor versions.
- Golden stores from every supported version are opened by the current binary
  or upgraded by a no-clobber, interruption-safe migration command with a
  tested rollback guide.
- Recovery after supervisor or machine interruption reports ownership
  uncertainty explicitly and never claims cleanup, readiness, or terminal
  state without durable evidence.
- A platform limitation or degraded cleanup primitive is explicit and never
  silently weakens the ownership contract.

### Lifecycle correctness and security

- Cross-platform stress completes at least 10,000 aggregate start, readiness,
  wait, log, stop, timeout, failure, recovery, and GC lifecycles without a
  surviving owned descendant, hung control operation, or false terminal state.
- The lease stress corpus has zero duplicate live port or temporary-directory
  grants, zero reuse before durable release, and zero placeholder resolution
  outside the owning run.
- The adversarial store corpus has 100% rejection of token substitution,
  symlinked control files, malformed state transitions, log corruption, stale
  stop requests, unsafe GC targets, and cross-run identity mismatches.
- Log pressure fixtures preserve complete-stream digests and exact
  `captured_bytes`/`dropped_bytes` accounting even when retained records are
  truncated at every configured boundary.
- An independent security review covers store permissions, owner tokens,
  process groups, Windows Job Objects, probes, leases, path handling, symlinks,
  recovery races, logs, and deletion boundaries; all critical and high findings
  are resolved.
- No known critical or high-severity vulnerability is open at release time.

### Performance and bounds

- Start-to-supervisor-running control overhead remains below 250 ms p95,
  excluding child startup and configured readiness checks, on the published
  runner.
- Status, wait-on-completed-run, bounded logs, leases, and list operations
  remain below 250 ms p95 on the published 1,000-run store corpus.
- Each idle supervisor remains below 128 MiB peak resident memory, and retained
  log storage never exceeds its configured byte bound plus documented constant
  record metadata.
- Runtime, readiness, probe, log, stop-grace, list, and GC work never exceed
  configured bounds without an explicit structured state.
- Corpus definitions, runner images, raw measurements, and regression
  thresholds are versioned with the repository.

### Delivery and maintenance

- Required CI and lifecycle stress remain green on Linux, macOS, and Windows
  for 30 consecutive days before the v1.0 tag.
- Releases originate only from protected `main` and signed annotated tags; all
  native archives have verified checksums, GitHub-hosted provenance, and a
  CycloneDX SBOM attestation.
- The release, recovery, and process-leak incident runbooks are exercised by
  two maintainers, or governance records the single-maintainer continuity risk
  and a tested recovery procedure.
- Security reports are acknowledged within 3 business days and receive an
  initial assessment within 7.

### Adoption evidence

- At least three independent external workflows are recorded in
  [ADOPTERS.md](ADOPTERS.md) with the lifecycle or coordination decision
  ProcHerd improved.
- At least two adopters report repeat use separated by 30 days.
- At least one public integration uses durable status, readiness, logs, or
  leases to control a real dev server, watcher, test service, or build.
- At least one non-maintainer issue, discussion, stress fixture, documentation
  change, test, platform fix, or code contribution is resolved and credited.

Maintainer-authored fixtures, automated downloads, stars, and synthetic
accounts cannot satisfy adoption gates.
