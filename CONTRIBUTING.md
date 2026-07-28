# Contributing to ProcHerd

Contributions of code, cross-platform fixtures, security hardening,
documentation, benchmarks, integrations, and reproducible bug reports are
welcome.

## Before opening an issue

- Use GitHub Discussions for usage questions and design exploration.
- Search existing issues and reduce process bugs to a synthetic child program.
- Report security-sensitive behavior privately through [SECURITY.md](SECURITY.md).
- Remove secrets, private paths, environment values, state tokens, and logs.

## Development setup

ProcHerd requires Rust 1.85 or newer.

```bash
git clone https://github.com/yhay81/procherd.git
cd procherd
cargo test --all-targets --locked
```

Process-tree behavior differs by operating system. Cross-platform lifecycle
changes must be exercised in GitHub Actions, not inferred from one host.

Persisted run-state parsing is continuously fuzzed. See
[FUZZING.md](FUZZING.md) for the reproducible local command and crash-handling
rules.

## Making a change

1. Open an issue first for a schema, lifecycle, state-layout, security-boundary,
   process-control, or lease-guarantee change. Small fixes do not require one.
2. Keep the public command an argument-vector contract.
3. Preserve bounded logs, explicit loss, local-only readiness, conservative
   deletion, idempotent cleanup, and honest platform guarantees.
4. Add a success case and the relevant failure, timeout, race, or cleanup case.
5. Update schemas, docs, platform notes, and the changelog for public behavior.
6. Run:

   ```bash
   cargo fmt --check
   cargo clippy --all-targets --locked -- -D warnings
   cargo test --all-targets --locked
   cargo package --locked --allow-dirty
   ```

## Public contracts

Commands and flags, run IDs, JSON and NDJSON documents, schema identifiers,
exit codes, lifecycle/readiness/exit enums, cursor semantics, lease guarantees,
state layout, and deletion rules are public interfaces. Breaking changes
require explicit migration notes and a versioned contract.

## Pull requests

Explain the user problem, smallest complete scope, operating-system behavior,
guarantee limits, exact verification, and failure paths. By contributing, you
agree that your contribution is licensed under MIT and follows the
[Code of Conduct](CODE_OF_CONDUCT.md).
