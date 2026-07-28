# Changelog

All notable ProcHerd changes are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and releases use
semantic versioning.

## [Unreleased]

### Changed

- Upgraded `sha2` to 0.11 and centralized lowercase hexadecimal encoding while
  preserving owner-token and digest wire formats.
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

[Unreleased]: https://github.com/yhay81/procherd/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/yhay81/procherd/releases/tag/v0.1.0
