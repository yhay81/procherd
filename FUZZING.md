# Fuzzing ProcHerd

ProcHerd fuzzes its persisted run-state boundary with
AddressSanitizer. The `persisted_run_state` target exercises the production
document bound, strict typed JSON parser, schema check, canonical ULID
reconstruction, and persisted lease-name validation before filesystem use.

To fuzz a pull request locally, check out its branch and run:

```bash
rustup toolchain install nightly --profile minimal
cargo install cargo-fuzz --version 0.13.2 --locked
mkdir -p fuzz/corpus/persisted_run_state
cp tests/fixtures/contracts/v0.1/run_01J00000000000000000000000/state.json \
  fuzz/corpus/persisted_run_state/
cargo +nightly fuzz run persisted_run_state -- -max_total_time=300
```

The local run spends five minutes fuzzing after compilation. Omit
`-max_total_time=300` for a longer session. Pull requests run the deterministic
persisted-state compatibility corpus against the current code. The full
AddressSanitizer fuzz run is optional.

The comparison build, 15-minute batch, and corpus pruning are manual. Select
the corresponding ClusterFuzzLite workflow in GitHub Actions and choose
**Run workflow** on `main` to run it with GitHub artifacts. A completed batch
can publish findings to code scanning. Hosted jobs have a one-minute limit,
so use the local commands below for a full session.

After setting up cargo-fuzz and the seed corpus above, run these commands
from the repository root as needed:

```bash
cargo +nightly fuzz build persisted_run_state
cargo +nightly fuzz run persisted_run_state -- -max_total_time=900
cargo +nightly fuzz cmin persisted_run_state
```

`cmin` minimizes the local target corpus in place. Make a copy first if
you need the original inputs.

Persisted state can disclose commands, environment names, paths, and process
metadata. Keep minimized crashes private until reviewed, add a deterministic
regression test, and use [SECURITY.md](SECURITY.md) for security-relevant
findings.
