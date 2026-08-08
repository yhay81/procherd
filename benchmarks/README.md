# ProcHerd performance baseline

This directory defines and enforces ProcHerd's reproducible v1.0 performance
thresholds on pull requests and in the weekly scheduled benchmark. The harness
also enforces the 128 MiB log-pressure supervisor safety bound.

## Workloads

`generate_store.py` expands the published v0.1 terminal-store fixture into
1,000 valid, inactive runs with deterministic ULIDs, timestamps, JSON,
NDJSON, permissions, and a content digest. The generated corpus and scripts are
synthetic project artifacts covered by the repository's MIT license.

Each raw sample performs untimed build and corpus setup. The workflow discards
one warm-up and then captures 20 samples, each in this fixed order:

1. start-to-running wall time for a minimal bundled child fixture;
2. supervisor RSS and high-water RSS while draining a deterministic 256 MiB
   stream into a one-byte retention budget;
3. status, wait-on-completed-run, one-record logs, and leases for a
   representative run in the 1,000-run store;
4. complete list of the 1,000-run store.

The start measurement is a conservative end-to-end observation: it includes
the minimal child spawn even though the v1.0 control-overhead target excludes
application startup. While that child is idle, Linux `/proc` provides the
supervisor's current RSS and high-water RSS before the harness stops it.

Raw results record GNU `time` wall time and peak resident memory for each CLI
process, output bytes, idle and log-pressure supervisor memory, exact log
capture/drop accounting and digest evidence, fixture counts and digest, runner
identity, semantic result evidence, and the exact ProcHerd commit.

## Enforced thresholds

The versioned policy in `thresholds.json` enforces:

- start-to-running, status, wait, bounded logs, leases, and list below
  250 milliseconds p95;
- idle supervisor high-water RSS no greater than 128 MiB;
- control-process peak RSS no greater than 256 MiB.

The start measurement includes child startup, making its 250 ms limit stricter
than the ROADMAP's control-overhead-only limit. Twenty samples make
nearest-rank p95 the second-slowest observation. Once
`baseline-ubuntu24.json` is present, metrics must also remain within the
stricter of the absolute limit and the versioned noise allowance: 1.5 times
baseline or baseline plus 50 ms for time and 16 MiB for memory.

## Run

The supported measurement environment is the `ubuntu-24.04` x86_64
GitHub-hosted runner selected by `.github/workflows/benchmark.yml`. Run one raw
sample on a compatible Linux machine with:

```bash
benchmarks/run.sh benchmark-results.json
jq . benchmark-results.json
```

Run evaluator tests with:

```bash
python3 -m unittest benchmarks/test_evaluate.py
```

GNU `time`, GNU `stat`, `timeout`, `/proc`, `jq`, Python 3, Git, Cargo, and the
locked Rust dependency graph are required. Build and corpus-generation time are
excluded. Generated stores and process logs are temporary and are not uploaded.

The workflow uploads all 20 raw samples and the aggregate evaluation for 90
days. The checked-in baseline is refreshed only from a successful protected
runner evaluation, so baseline changes remain reviewable.
