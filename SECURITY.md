# Security policy

## Supported versions

ProcHerd is pre-1.0. Security fixes are applied to the latest tagged release.
Older pre-1.0 releases are unsupported after a newer release is available.

| Version | Supported |
| --- | --- |
| Latest tagged release | Yes |
| Older pre-1.0 releases | No |
| Unreleased development builds | Best effort |

## Reporting a vulnerability

Use
[GitHub private vulnerability reporting](https://github.com/yhay81/procherd/security/advisories/new).
Do not open a public issue for command execution, path traversal, symlink,
owner-token, environment leakage, process escape, readiness SSRF, port lease,
log disclosure, integrity, or unsafe-deletion vulnerabilities.

Include the ProcHerd version, operating system, exact redacted command,
relevant structured state, and a minimal synthetic child program. Do not
attach real logs or state directories; they may contain secrets and private
paths.

Acknowledgement is targeted within 7 days. The maintainer will validate the
report, coordinate disclosure, add a regression test, and publish a GitHub
Security Advisory when appropriate. These are volunteer-project targets, not a
service-level agreement.

## Trust and containment boundaries

- ProcHerd executes caller-selected code with an inherited environment. It
  does not sandbox network, filesystem, CPU, memory, or process access.
- Commands are spawned as argument arrays. A shell runs only when explicitly
  selected as the program.
- Unix process groups and Windows Job Objects provide best-available tree
  ownership, not a defense against privileged or deliberately escaping code.
- State contains command arguments, working directories, readiness evidence,
  resource paths, and raw bounded logs. Logs are not redacted or encrypted.
- Environment values are not persisted. The count and SHA-256 digest of
  inherited variable names are recorded; lease-injected names are recorded.
- New Unix state paths are owner-only. Windows access follows the containing
  directory ACL. Anyone able to read the state directory can inspect runs and
  obtain the owner token used for mutation.
- HTTP/TCP probes are restricted to loopback. HTTP is plaintext, accepts 2xx
  and 3xx, sends no credentials, and has bounded connection/read/write times.
- Port leases avoid collisions among cooperating ProcHerd runs but cannot
  prevent an unrelated process from binding during the recorded handoff gap.
- GC is dry-run by default and deletes only revalidated terminal, inactive,
  canonical run directories. State symlinks are rejected on read.
- After a supervisor or machine crash, a child may outlive accurate durable
  state. Version 0.1 detects inactive-supervisor/live-state disagreement but
  does not promise reattachment or automatic cleanup.

## Release and dependency policy

Dependabot monitors Rust and GitHub Actions dependencies. CI checks
`Cargo.lock` against RustSec advisories. Tagged releases are built only by the
release workflow and include checksums, CycloneDX SBOMs, and GitHub/Sigstore
attestations. See [RELEASING.md](RELEASING.md).
