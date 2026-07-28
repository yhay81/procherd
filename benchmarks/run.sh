#!/usr/bin/env bash
set -euo pipefail

export LC_ALL=C

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
result_path="${1:-${root_dir}/benchmark-results.json}"
binary="${root_dir}/target/release/procherd"
generator="${root_dir}/benchmarks/generate_store.py"

for dependency in awk cargo git grep jq python3 stat timeout uname; do
  command -v "${dependency}" >/dev/null || {
    printf 'missing benchmark dependency: %s\n' "${dependency}" >&2
    exit 1
  }
done

if ! /usr/bin/time --version 2>&1 | grep -qi 'GNU time'; then
  printf 'benchmarks/run.sh requires GNU /usr/bin/time (the Ubuntu runner provides it)\n' >&2
  exit 1
fi

temp_dir="$(mktemp -d)"
store_1000="${temp_dir}/store-1000"
start_store="${temp_dir}/start-store"
fixture_metadata="${temp_dir}/fixture.json"
start_run=""

cleanup() {
  if [[ -n "${start_run}" && -d "${start_store}" && -x "${binary}" ]]; then
    "${binary}" \
      --state-dir "${start_store}" \
      --format json \
      stop "${start_run}" --grace 1s >/dev/null 2>&1 || true
  fi
  rm -rf "${temp_dir}"
}
trap cleanup EXIT

cd "${root_dir}"
cargo build --release --locked
python3 "${generator}" \
  --runs 1000 \
  --output "${store_1000}" >"${fixture_metadata}"
target_run="$(jq -r .last_run_id "${fixture_metadata}")"

measure() {
  local metrics_path="$1"
  local output_path="$2"
  shift 2

  /usr/bin/time \
    -f '{"wall_seconds": %e, "max_rss_kib": %M, "exit_code": %x}' \
    -o "${metrics_path}" \
    timeout --signal=KILL 45s "$@" >"${output_path}"
  jq -e . "${metrics_path}" >/dev/null
  jq -e . "${output_path}" >/dev/null
}

start_metrics="${temp_dir}/start.metrics.json"
start_output="${temp_dir}/start.output.json"
marker="${temp_dir}/idle.marker"
measure "${start_metrics}" "${start_output}" \
  "${binary}" \
  --state-dir "${start_store}" \
  --format json \
  start \
  --max-runtime 30s \
  --runtime-grace 1s \
  -- \
  "${binary}" __fixture touch --path "${marker}"

start_run="$(jq -r .run.run_id "${start_output}")"
supervisor_pid="$(jq -r .run.process.supervisor_pid "${start_output}")"
supervisor_rss_kib="$(
  awk '/^VmRSS:/ { print $2 }' "/proc/${supervisor_pid}/status"
)"
supervisor_hwm_kib="$(
  awk '/^VmHWM:/ { print $2 }' "/proc/${supervisor_pid}/status"
)"
test "${supervisor_rss_kib}" -gt 0
test "${supervisor_hwm_kib}" -gt 0
"${binary}" \
  --state-dir "${start_store}" \
  --format json \
  stop "${start_run}" --grace 1s >/dev/null
start_run=""

status_metrics="${temp_dir}/status.metrics.json"
status_output="${temp_dir}/status.output.json"
wait_metrics="${temp_dir}/wait.metrics.json"
wait_output="${temp_dir}/wait.output.json"
logs_metrics="${temp_dir}/logs.metrics.json"
logs_output="${temp_dir}/logs.output.json"
leases_metrics="${temp_dir}/leases.metrics.json"
leases_output="${temp_dir}/leases.output.json"
list_metrics="${temp_dir}/list.metrics.json"
list_output="${temp_dir}/list.output.json"

measure "${status_metrics}" "${status_output}" \
  "${binary}" --state-dir "${store_1000}" --format json \
  status "${target_run}"
measure "${wait_metrics}" "${wait_output}" \
  "${binary}" --state-dir "${store_1000}" --format json \
  wait "${target_run}" --for exit --timeout 1s
measure "${logs_metrics}" "${logs_output}" \
  "${binary}" --state-dir "${store_1000}" --format json \
  logs "${target_run}" --after 0 --limit 1
measure "${leases_metrics}" "${leases_output}" \
  "${binary}" --state-dir "${store_1000}" --format json \
  leases "${target_run}"
measure "${list_metrics}" "${list_output}" \
  "${binary}" --state-dir "${store_1000}" --format json list

mkdir -p "$(dirname "${result_path}")"
jq -n \
  --arg generated_at "$(date -u +'%Y-%m-%dT%H:%M:%SZ')" \
  --arg git_sha "$(git rev-parse HEAD)" \
  --arg runner_os "${RUNNER_OS:-Linux}" \
  --arg runner_arch "$(uname -m)" \
  --arg runner_image "${ImageOS:-unknown}" \
  --arg runner_image_version "${ImageVersion:-unknown}" \
  --argjson supervisor_rss_kib "${supervisor_rss_kib}" \
  --argjson supervisor_hwm_kib "${supervisor_hwm_kib}" \
  --argjson start_output_bytes "$(stat -c '%s' "${start_output}")" \
  --argjson status_output_bytes "$(stat -c '%s' "${status_output}")" \
  --argjson wait_output_bytes "$(stat -c '%s' "${wait_output}")" \
  --argjson logs_output_bytes "$(stat -c '%s' "${logs_output}")" \
  --argjson leases_output_bytes "$(stat -c '%s' "${leases_output}")" \
  --argjson list_output_bytes "$(stat -c '%s' "${list_output}")" \
  --slurpfile fixture "${fixture_metadata}" \
  --slurpfile start_metrics "${start_metrics}" \
  --slurpfile start_output "${start_output}" \
  --slurpfile status_metrics "${status_metrics}" \
  --slurpfile status_output "${status_output}" \
  --slurpfile wait_metrics "${wait_metrics}" \
  --slurpfile wait_output "${wait_output}" \
  --slurpfile logs_metrics "${logs_metrics}" \
  --slurpfile logs_output "${logs_output}" \
  --slurpfile leases_metrics "${leases_metrics}" \
  --slurpfile leases_output "${leases_output}" \
  --slurpfile list_metrics "${list_metrics}" \
  --slurpfile list_output "${list_output}" \
  '{
    schema_version: "procherd.benchmark.v1",
    generated_at: $generated_at,
    git_sha: $git_sha,
    runner: {
      os: $runner_os,
      arch: $runner_arch,
      image: $runner_image,
      image_version: $runner_image_version
    },
    fixture: $fixture[0],
    idle_supervisor: {
      rss_kib: $supervisor_rss_kib,
      high_water_kib: $supervisor_hwm_kib
    },
    measurements: [
      {
        id: "start_to_running",
        process: $start_metrics[0],
        output_bytes: $start_output_bytes,
        result: {
          schema_version: $start_output[0].schema_version,
          status: $start_output[0].run.status,
          supervisor_active: $start_output[0].run.supervisor_active
        }
      },
      {
        id: "status_1000_store",
        process: $status_metrics[0],
        output_bytes: $status_output_bytes,
        result: {
          schema_version: $status_output[0].schema_version,
          status: $status_output[0].run.status,
          observed_status: $status_output[0].run.observed_status
        }
      },
      {
        id: "wait_completed_1000_store",
        process: $wait_metrics[0],
        output_bytes: $wait_output_bytes,
        result: {
          schema_version: $wait_output[0].schema_version,
          condition: $wait_output[0].condition,
          status: $wait_output[0].run.status
        }
      },
      {
        id: "logs_bounded_1000_store",
        process: $logs_metrics[0],
        output_bytes: $logs_output_bytes,
        result: {
          schema_version: $logs_output[0].schema_version,
          records: ($logs_output[0].records | length),
          has_more: $logs_output[0].has_more,
          terminal: $logs_output[0].terminal
        }
      },
      {
        id: "leases_1000_store",
        process: $leases_metrics[0],
        output_bytes: $leases_output_bytes,
        result: {
          schema_version: $leases_output[0].schema_version,
          ports: ($leases_output[0].leases.ports | length),
          temp_directories:
            ($leases_output[0].leases.temp_directories | length)
        }
      },
      {
        id: "list_1000_store",
        process: $list_metrics[0],
        output_bytes: $list_output_bytes,
        result: {
          schema_version: $list_output[0].schema_version,
          runs: ($list_output[0].runs | length)
        }
      }
    ],
    derived: {
      max_control_process_peak_rss_mib:
        ([
          $start_metrics[0].max_rss_kib,
          $status_metrics[0].max_rss_kib,
          $wait_metrics[0].max_rss_kib,
          $logs_metrics[0].max_rss_kib,
          $leases_metrics[0].max_rss_kib,
          $list_metrics[0].max_rss_kib
        ] | max | . / 1024),
      idle_supervisor_high_water_mib: ($supervisor_hwm_kib / 1024)
    },
    threshold_status: "observation_only"
  }' >"${result_path}"

jq -e '
  .schema_version == "procherd.benchmark.v1"
  and .fixture.runs == 1000
  and .idle_supervisor.rss_kib > 0
  and .idle_supervisor.high_water_kib > 0
  and all(
    .measurements[];
    .process.exit_code == 0
      and .process.wall_seconds >= 0
      and .process.max_rss_kib > 0
      and .output_bytes > 0
  )
  and any(
    .measurements[];
    .id == "start_to_running"
      and .result.status == "running"
      and .result.supervisor_active
  )
  and any(
    .measurements[];
    .id == "status_1000_store"
      and .result.status == "exited"
      and .result.observed_status == "consistent"
  )
  and any(
    .measurements[];
    .id == "wait_completed_1000_store"
      and .result.condition == "exit"
      and .result.status == "exited"
  )
  and any(
    .measurements[];
    .id == "logs_bounded_1000_store"
      and .result.records == 1
      and .result.terminal
  )
  and any(
    .measurements[];
    .id == "list_1000_store"
      and .result.runs == 1000
  )
' "${result_path}" >/dev/null

printf 'wrote %s\n' "${result_path}"
