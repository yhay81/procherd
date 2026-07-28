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
- Published latency, cleanup, log-pressure, and readiness benchmarks.

## 0.3 integrations and policy

- Library-facing orchestration adapter with cancellation-safe ownership.
- Opt-in environment allow/deny policy without recording values.
- Config-file/spec input after a security and compatibility design review.
- Configurable log retention strategies that preserve explicit loss.
- More readiness request controls without allowing remote probes by default.

## 1.0 gates

- Stable command, JSON, exit-code, and store compatibility policy.
- Documented migration support across two consecutive pre-1.0 minor releases.
- Cross-platform cleanup and lease stress benchmark with published results.
- Independent security review of state, tokens, process control, probes, and
  deletion boundaries.
- Reproducible native releases with checksums, SBOMs, and provenance.
- At least three verified, opt-in external workflows in
  [ADOPTERS.md](ADOPTERS.md).
- Maintainer runbook, succession path, support policy, and no open critical
  correctness or security defects.
