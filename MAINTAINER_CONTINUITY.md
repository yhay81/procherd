# Maintainer continuity

ProcHerd currently has one repository owner and one release-capable maintainer,
`@yhay81`. This is an explicit continuity risk, not evidence of second-person
review or a promise that private project state can be recovered.

## Public recovery boundary

The monthly `Maintainer continuity drill` workflow and
`scripts/continuity-drill.sh` prove that a person with no repository write
credential can recover and validate:

- a complete public Git mirror with a successful `git fsck`;
- every published `v*` tag against signing subkey fingerprint
  `0C153FFE2B0274365ACB1BF1AEFA86FA828C52C5`;
- every latest-release asset against `SHA256SUMS`;
- GitHub build-provenance and CycloneDX SBOM attestations for the native
  archive selected for the drill host;
- the released native binary by running `procherd --version`.

The public key is downloaded from the maintainer's GitHub identity but accepted
only when the pinned signing fingerprint is present. A changed or missing key
fails closed.

Run the drill locally with public read access:

```bash
GH_TOKEN="$(gh auth token)" \
  ./scripts/continuity-drill.sh yhay81/procherd procherd
```

The automated drill runs on Linux x86_64; local macOS drills select arm64 or
x86_64 to match the host. Temporary recovery data is deleted on exit.

## State that public recovery does not restore

The drill cannot recover GitHub account or repository administration, private
vulnerability reports, Actions secrets, environment approvals, the private
signing key, or package-registry ownership and credentials.

Loss of the signing key requires a reviewed governance change that records the
old and new fingerprints, revocation when possible, an update to this document,
and a new patch version. Existing tags and releases remain immutable.

Loss of repository control is not solved by a public clone. Account recovery
comes first. A clearly named fork may use the verified public history only when
control cannot be restored, and must not claim continuity of repository
identity, private reports, or registry ownership.

## v1.0 gate

Before v1.0, either a second trusted maintainer independently exercises release,
recovery, and process-leak incident runbooks, or the single-maintainer risk
remains disclosed while this drill plus repository-account and
package-registry recovery are tested and recorded without storing secrets in
Git. Until then, continuity remains open.
