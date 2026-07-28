# Fuzzing ProcHerd

ProcHerd continuously fuzzes its persisted run-state boundary with
AddressSanitizer. The `persisted_run_state` target exercises the production
document bound, strict typed JSON parser, schema check, canonical ULID
reconstruction, and persisted lease-name validation before filesystem use.

Install a current nightly toolchain and the pinned local runner, then run:

```bash
cargo install cargo-fuzz --version 0.13.2 --locked
mkdir -p fuzz/corpus/persisted_run_state
cp tests/fixtures/contracts/v0.1/run_01J00000000000000000000000/state.json \
  fuzz/corpus/persisted_run_state/
cargo +nightly fuzz run persisted_run_state
```

Pull requests receive a five-minute ClusterFuzzLite code-change run. A
15-minute batch run executes weekly on `main`, seeded by the versioned
run-state fixture, and publishes machine-readable findings to GitHub code
scanning.
Each code-changing `main` update also saves a comparison build so later pull
requests can distinguish newly introduced crashes. The accumulated corpus is
pruned after every weekly batch.

Persisted state can disclose commands, environment names, paths, and process
metadata. Keep minimized crashes private until reviewed, add a deterministic
regression test, and use [SECURITY.md](SECURITY.md) for security-relevant
findings.
