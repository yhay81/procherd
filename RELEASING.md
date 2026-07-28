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
   git tag -s v0.2.0 -m "ProcHerd 0.2.0"
   git push origin v0.2.0
   ```

7. The release workflow creates native archives, completions, a CycloneDX
   SBOM, `SHA256SUMS`, a GitHub release, and GitHub/Sigstore build-provenance
   and SBOM attestations. Each archive includes a downloadable
   `.intoto.jsonl` provenance bundle for local verification.
8. In a clean directory, verify downloads:

   ```bash
   sha256sum --check SHA256SUMS
   gh attestation verify procherd-v0.2.0-linux-x86_64.tar.gz \
     --repo yhay81/procherd
   gh attestation verify procherd-v0.2.0-linux-x86_64.tar.gz \
     --repo yhay81/procherd \
     --bundle procherd-v0.2.0-linux-x86_64.tar.gz.intoto.jsonl \
     --signer-workflow yhay81/procherd/.github/workflows/release.yml
   gh attestation verify procherd-v0.2.0-linux-x86_64.tar.gz \
     --repo yhay81/procherd \
     --predicate-type https://cyclonedx.org/bom
   ```

9. Extract every archive and run `procherd --version`, completion generation,
   brief schema emission, and a platform-native lifecycle fixture.
10. Release notes must link to installation, checksums, SBOM, provenance,
    changelog, platform guarantees, state/log sensitivity, and private security
    reporting.

Publishing to crates.io remains manual until registry ownership and credentials
are configured:

```bash
cargo publish --locked
```

Never move or reuse a published tag or version. A failed release is followed
by a documented patch release.
