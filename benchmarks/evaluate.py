#!/usr/bin/env python3
"""Aggregate benchmark samples and enforce versioned performance thresholds."""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import sys
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


class EvaluationError(ValueError):
    """Raised when benchmark evidence is incomplete or inconsistent."""


def load_json(path: Path) -> Any:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise EvaluationError(f"cannot load {path}: {error}") from error


def finite_number(value: Any, context: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise EvaluationError(f"{context} must be a number")
    number = float(value)
    if not math.isfinite(number) or number < 0:
        raise EvaluationError(f"{context} must be finite and non-negative")
    return number


def nearest_rank(values: list[float], percentile: float) -> float:
    if not values:
        raise EvaluationError("cannot calculate a percentile without samples")
    if not 0 < percentile <= 1:
        raise EvaluationError("percentile must be in (0, 1]")
    ordered = sorted(values)
    rank = math.ceil(percentile * len(ordered))
    return ordered[rank - 1]


def measurement(sample: dict[str, Any], measurement_id: str) -> dict[str, Any]:
    matches = [
        item
        for item in sample.get("measurements", [])
        if item.get("id") == measurement_id
    ]
    if len(matches) != 1:
        raise EvaluationError(
            f"expected exactly one measurement named {measurement_id!r}"
        )
    return matches[0]


def source_value(sample: dict[str, Any], source: dict[str, Any]) -> float:
    kind = source.get("kind")
    if kind == "measurement_process":
        item = measurement(sample, source["measurement_id"])
        process = item.get("process")
        if not isinstance(process, dict):
            raise EvaluationError("measurement process is missing")
        value = process.get(source["field"])
    elif kind == "derived":
        derived = sample.get("derived")
        if not isinstance(derived, dict):
            raise EvaluationError("derived measurements are missing")
        value = derived.get(source["field"])
    else:
        raise EvaluationError(f"unsupported metric source kind: {kind!r}")
    scale = finite_number(source.get("scale", 1), "metric source scale")
    return finite_number(value, f"{kind} metric value") * scale


def common_identity(
    samples: list[dict[str, Any]], expected_schema: str
) -> tuple[str, dict[str, str]]:
    git_shas: set[str] = set()
    runners: set[tuple[str, str, str, str]] = set()
    for sample in samples:
        if sample.get("schema_version") != expected_schema:
            raise EvaluationError(
                f"expected sample schema {expected_schema!r}, "
                f"found {sample.get('schema_version')!r}"
            )
        git_sha = sample.get("git_sha")
        if not isinstance(git_sha, str) or len(git_sha) != 40:
            raise EvaluationError("sample git_sha must be a 40-character SHA")
        git_shas.add(git_sha)
        runner = sample.get("runner")
        if not isinstance(runner, dict):
            raise EvaluationError("sample runner identity is missing")
        runners.add(
            tuple(
                str(runner.get(field, ""))
                for field in ("os", "arch", "image", "image_version")
            )
        )
    if len(git_shas) != 1:
        raise EvaluationError("all samples must measure the same commit")
    if len(runners) != 1:
        raise EvaluationError("all samples must use the same runner image")
    runner_tuple = runners.pop()
    return git_shas.pop(), dict(
        zip(("os", "arch", "image", "image_version"), runner_tuple, strict=True)
    )


def baseline_metrics(path: Path | None) -> tuple[dict[str, float], dict[str, Any] | None]:
    if path is None:
        return {}, None
    baseline = load_json(path)
    if baseline.get("schema_version") != "benchmark.evaluation.v1":
        raise EvaluationError("baseline must use benchmark.evaluation.v1")
    if not baseline.get("passed"):
        raise EvaluationError("baseline must be a passing evaluation")
    metrics = baseline.get("metrics")
    if not isinstance(metrics, list):
        raise EvaluationError("baseline metrics are missing")
    values: dict[str, float] = {}
    for metric in metrics:
        metric_id = metric.get("id")
        if not isinstance(metric_id, str) or metric_id in values:
            raise EvaluationError("baseline metric IDs must be unique strings")
        values[metric_id] = finite_number(
            metric.get("observed"), f"baseline metric {metric_id}"
        )
    return values, {
        "path": str(path),
        "benchmark_schema_version": baseline.get("benchmark_schema_version"),
        "generated_at": baseline.get("generated_at"),
        "git_sha": baseline.get("git_sha"),
        "runner": baseline.get("runner"),
        "sample_count": baseline.get("sample_count"),
        "config_sha256": baseline.get("config_sha256"),
    }


def evaluate(
    config_path: Path,
    sample_paths: list[Path],
    baseline_path: Path | None = None,
) -> dict[str, Any]:
    config_bytes = config_path.read_bytes()
    config = json.loads(config_bytes)
    if config.get("schema_version") != "benchmark.thresholds.v1":
        raise EvaluationError("config must use benchmark.thresholds.v1")
    expected_count = config.get("sample_count")
    if not isinstance(expected_count, int) or expected_count < 2:
        raise EvaluationError("sample_count must be an integer of at least 2")
    if len(sample_paths) != expected_count:
        raise EvaluationError(
            f"expected {expected_count} samples, found {len(sample_paths)}"
        )
    samples = [load_json(path) for path in sample_paths]
    expected_schema = config.get("benchmark_schema_version")
    if not isinstance(expected_schema, str):
        raise EvaluationError("benchmark_schema_version is required")
    git_sha, runner = common_identity(samples, expected_schema)
    required_runner = config.get("runner")
    if not isinstance(required_runner, dict):
        raise EvaluationError("config runner identity is required")
    for field in ("os", "arch", "image"):
        if runner[field] != required_runner.get(field):
            raise EvaluationError(
                f"runner {field} must be {required_runner.get(field)!r}, "
                f"found {runner[field]!r}"
            )

    config_sha256 = hashlib.sha256(config_bytes).hexdigest()
    prior_values, baseline = baseline_metrics(baseline_path)
    if baseline is not None:
        if baseline["benchmark_schema_version"] != expected_schema:
            raise EvaluationError("baseline benchmark schema does not match config")
        if baseline["sample_count"] != expected_count:
            raise EvaluationError("baseline sample count does not match config")
        if baseline["config_sha256"] != config_sha256:
            raise EvaluationError("baseline threshold config digest does not match")
        baseline_runner = baseline["runner"]
        if not isinstance(baseline_runner, dict):
            raise EvaluationError("baseline runner identity is missing")
        for field in ("os", "arch", "image"):
            if baseline_runner.get(field) != runner[field]:
                raise EvaluationError(
                    f"baseline runner {field} does not match current samples"
                )
    metric_configs = config.get("metrics")
    if not isinstance(metric_configs, list) or not metric_configs:
        raise EvaluationError("at least one metric is required")
    seen_ids: set[str] = set()
    results: list[dict[str, Any]] = []
    for metric_config in metric_configs:
        metric_id = metric_config.get("id")
        if not isinstance(metric_id, str) or metric_id in seen_ids:
            raise EvaluationError("metric IDs must be unique strings")
        seen_ids.add(metric_id)
        values = [
            source_value(sample, metric_config.get("source", {}))
            for sample in samples
        ]
        statistic = metric_config.get("statistic")
        if statistic == "p95":
            observed = nearest_rank(values, 0.95)
        elif statistic == "max":
            observed = max(values)
        else:
            raise EvaluationError(f"unsupported statistic: {statistic!r}")
        absolute_maximum = finite_number(
            metric_config.get("maximum"), f"metric {metric_id} maximum"
        )
        regression_limit = None
        effective_maximum = absolute_maximum
        regression = metric_config.get("regression")
        if baseline_path is not None:
            if metric_id not in prior_values:
                raise EvaluationError(f"baseline is missing metric {metric_id!r}")
            if not isinstance(regression, dict):
                raise EvaluationError(
                    f"metric {metric_id!r} needs a regression policy"
                )
            ratio = finite_number(regression.get("max_ratio"), "regression max_ratio")
            tolerance = finite_number(
                regression.get("absolute_tolerance"),
                "regression absolute_tolerance",
            )
            prior = prior_values[metric_id]
            regression_limit = max(prior * ratio, prior + tolerance)
            effective_maximum = min(absolute_maximum, regression_limit)
        passed = observed <= effective_maximum
        results.append(
            {
                "id": metric_id,
                "description": metric_config.get("description"),
                "statistic": statistic,
                "unit": metric_config.get("unit"),
                "samples": values,
                "observed": observed,
                "absolute_maximum": absolute_maximum,
                "baseline_observed": prior_values.get(metric_id),
                "regression_limit": regression_limit,
                "effective_maximum": effective_maximum,
                "passed": passed,
            }
        )

    return {
        "schema_version": "benchmark.evaluation.v1",
        "generated_at": datetime.now(timezone.utc)
        .replace(microsecond=0)
        .isoformat()
        .replace("+00:00", "Z"),
        "benchmark_schema_version": expected_schema,
        "git_sha": git_sha,
        "runner": runner,
        "warmup_count": config.get("warmup_count"),
        "sample_count": len(samples),
        "percentile_method": "nearest_rank",
        "config_sha256": config_sha256,
        "baseline": baseline,
        "metrics": results,
        "passed": all(metric["passed"] for metric in results),
        "threshold_status": "enforced",
    }


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", required=True, type=Path)
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("samples", nargs="+", type=Path)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    try:
        result = evaluate(args.config, args.samples, args.baseline)
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(
            json.dumps(result, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )
    except (EvaluationError, OSError, json.JSONDecodeError, KeyError) as error:
        print(f"benchmark evaluation failed: {error}", file=sys.stderr)
        return 2
    if not result["passed"]:
        for metric in result["metrics"]:
            if not metric["passed"]:
                print(
                    f"{metric['id']}: observed {metric['observed']} "
                    f"exceeds {metric['effective_maximum']} {metric['unit']}",
                    file=sys.stderr,
                )
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
