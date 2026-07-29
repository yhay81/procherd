# Releasing ProcHerd

Only a release manager named in [GOVERNANCE.md](GOVERNANCE.md) may release.

1. Confirm the version is unpublished and `CHANGELOG.md`, `Cargo.toml`, and
   `Cargo.lock` agree.
2. Confirm the release commit is on `main`, the worktree is clean, and every
   required check passes.
3. Run:

   ```bash
   cargo fmt --check
   cargo clippy --all-targets --locked -- -D warnings
   cargo test --all-targets --locked
   cargo package --locked --allow-dirty
   cargo build --release --locked
   target/release/procherd --format json schema --document brief
   ```

4. Dogfood a successful ready run, a spawn failure, a readiness timeout, log
   overflow, runtime-limit cleanup, and idempotent stop. Confirm terminal
   commands report an inactive supervisor and no surviving fixture descendant.
5. Confirm Linux, macOS, Windows, Rust 1.85, RustSec audit, schemas,
   documentation links, and package contents in CI.
6. Create and push a signed annotated tag:

   ```bash
   git tag -s v0.3.0 -m "ProcHerd 0.3.0"
   git push origin v0.3.0
   ```

7. The release workflow creates native archives, completions, a CycloneDX
   SBOM, `SHA256SUMS`, a GitHub release, and GitHub/Sigstore build-provenance
   and SBOM attestations. Each archive includes a downloadable
   `.intoto.jsonl` provenance bundle for local verification.
8. In a clean directory, verify downloads:

   ```bash
   sha256sum --check SHA256SUMS
   gh attestation verify procherd-v0.3.0-linux-x86_64.tar.gz \
     --repo yhay81/procherd
   gh attestation verify procherd-v0.3.0-linux-x86_64.tar.gz \
     --repo yhay81/procherd \
     --bundle procherd-v0.3.0-linux-x86_64.tar.gz.intoto.jsonl \
     --signer-workflow yhay81/procherd/.github/workflows/release.yml
   gh attestation verify procherd-v0.3.0-linux-x86_64.tar.gz \
     --repo yhay81/procherd \
     --predicate-type https://cyclonedx.org/bom
   ```

9. Extract every archive and run `procherd --version`, completion generation,
   brief schema emission, and a platform-native lifecycle fixture.
10. Release notes must link to installation, checksums, SBOM, provenance,
    changelog, platform guarantees, state/log sensitivity, and private security
    reporting.

## crates.io

The first crates.io release must be published manually because Trusted
Publishing can only be configured after the crate exists. From the exact signed
release commit, repeat `cargo publish --dry-run --locked`, review
`cargo package --list --locked`, then publish:

```bash
cargo publish --locked
```

Use a Cargo credential provider backed by the operating-system credential
store. Never put a crates.io token in Git, workflow YAML, logs, or a
repository-level Actions secret. If Cargo times out after upload, check the
crates.io page and index before retrying; an accepted version is immutable.

After the first manual release:

1. Add the crate's Trusted Publisher in crates.io, restricted to
   `yhay81/procherd`, the dedicated publish workflow filename, and the protected
   `crates-io` GitHub environment.
2. Add that workflow only after the mapping exists. Grant only
   `contents: read` and `id-token: write`, pin every action to an immutable
   commit, exchange OIDC with `rust-lang/crates-io-auth-action`, and run
   `cargo publish --locked`.
3. Remove any temporary API token, verify registry ownership and account
   recovery without recording secrets, and require environment approval for
   every publish.
4. Install the exact version from crates.io in a clean environment and repeat
   the CLI smoke checks.

Never move or reuse a published tag or version. A failed release is followed
by a documented patch release.
