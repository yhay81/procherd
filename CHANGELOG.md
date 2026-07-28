# Changelog

All notable ProcHerd changes are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and releases use
semantic versioning.

## [Unreleased]

### Added

- Added a privacy-conscious adoption report form that captures evaluation,
  repeat-use, limitations, evidence, and public-listing permission.

## [0.2.0] - 2026-07-29

### Compatibility

- Preserved the public v0.1 CLI, lifecycle, store, log, and owner-token
  contracts. The v0.2 reader reopens the digest-pinned v0.1 run store
  byte-for-byte; no migration is required.

### Added

- Added a digest-pinned v0.1 golden run store with exact JSON/NDJSON round
  trips, full read-only lifecycle replay, and twelve declared corruption cases.
- Added deterministic start-to-running and 1,000-run-store benchmarks with
  weekly raw latency, output-size, controller-memory, and supervisor-memory
  artifacts.

### Changed

- Upgraded `sha2` to 0.11 and centralized lowercase hexadecimal encoding while
  preserving owner-token and digest wire formats.
- Durable state, nested store documents, and log records now reject unknown
  fields; log reads also validate Base64, byte counts, monotonic cursors, and
  cursor bounds before returning machine output.
- Defined measurable v1.0 compatibility, lifecycle and lease correctness,
  security, performance, delivery, maintenance, contribution, and
  repeat-adoption gates.

## [0.1.0] - 2026-07-28

### Added

- Detached per-run supervision with stable ULID run identity.
- Unix process-group and Windows Job Object ownership and cleanup.
- Durable start, status, list, wait, stop, lease, and conservative GC
  contracts.
- Bounded cursor-based stdout/stderr records, explicit dropped-byte counts,
  and complete-stream SHA-256 digests.
- AND-composed local TCP, local HTTP, regular-file, retained-log-literal, and
  named leased-port readiness with durable evidence.
- Coordinated loopback-port and private temporary-directory leases, argument
  placeholders, and injected environment variables.
- Maximum-runtime enforcement, structured failure reasons, stable exit-code
  classes, versioned JSON results, JSON Schemas, and shell completions.
- Cross-platform lifecycle, failure-path, lease, readiness, and GC tests.

[Unreleased]: https://github.com/yhay81/procherd/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/yhay81/procherd/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/yhay81/procherd/releases/tag/v0.1.0
