# ProcHerd performance baseline

This directory defines the reproducible, observation-only baseline used to
calibrate ProcHerd's v1.0 performance thresholds. Timing and memory are not yet
required pull-request checks.

## Workloads

`generate_store.py` expands the published v0.1 terminal-store fixture into
1,000 valid, inactive runs with deterministic ULIDs, timestamps, JSON,
NDJSON, permissions, and a content digest. The generated corpus and scripts are
synthetic project artifacts covered by the repository's MIT license.

The harness measures each operation once, without warm-up:

1. start-to-running wall time for a minimal bundled child fixture;
2. status, wait-on-completed-run, one-record logs, and leases for a
   representative run in the 1,000-run store;
3. complete list of the 1,000-run store.

The start measurement is a conservative end-to-end observation: it includes
the minimal child spawn even though the v1.0 control-overhead target excludes
application startup. While that child is idle, Linux `/proc` provides the
supervisor's current RSS and high-water RSS before the harness stops it.

Raw results record GNU `time` wall time and peak resident memory for each CLI
process, output bytes, supervisor memory, fixture counts and digest, runner
identity, semantic result evidence, and the exact ProcHerd commit.

## Run

The supported measurement environment is the `ubuntu-latest` GitHub-hosted
runner selected by `.github/workflows/benchmark.yml`. Run it manually with the
**Benchmark** workflow, or on a compatible Linux machine:

```bash
benchmarks/run.sh benchmark-results.json
jq . benchmark-results.json
```

GNU `time`, GNU `stat`, `timeout`, `/proc`, `jq`, Python 3, Git, Cargo, and the
locked Rust dependency graph are required. Build and corpus-generation time are
excluded. Generated stores and process logs are temporary and are not uploaded.

The workflow retains raw JSON for 90 days. Shared hosted runners are noisy, so
a single run is not a regression. Before enabling v1.0 gates, publish the
runner image, warm-up policy, sample count, p95 calculation, baseline window,
and noise-aware regression rule with the raw measurements.
