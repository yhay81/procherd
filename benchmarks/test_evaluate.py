import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


SPEC = importlib.util.spec_from_file_location(
    "benchmark_evaluate", Path(__file__).with_name("evaluate.py")
)
assert SPEC is not None and SPEC.loader is not None
evaluate_module = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(evaluate_module)


class EvaluateTests(unittest.TestCase):
    def sample(self, value: float, rss: float = 8.0) -> dict:
        return {
            "schema_version": "example.benchmark.v1",
            "git_sha": "a" * 40,
            "runner": {
                "os": "Linux",
                "arch": "x86_64",
                "image": "ubuntu24",
                "image_version": "test",
            },
            "measurements": [
                {
                    "id": "work",
                    "process": {
                        "wall_seconds": value,
                        "max_rss_kib": int(rss * 1024),
                    },
                }
            ],
            "derived": {"peak_rss_mib": rss},
        }

    def config(self) -> dict:
        return {
            "schema_version": "benchmark.thresholds.v1",
            "benchmark_schema_version": "example.benchmark.v1",
            "runner": {"os": "Linux", "arch": "x86_64", "image": "ubuntu24"},
            "warmup_count": 1,
            "sample_count": 20,
            "metrics": [
                {
                    "id": "wall",
                    "source": {
                        "kind": "measurement_process",
                        "measurement_id": "work",
                        "field": "wall_seconds",
                    },
                    "statistic": "p95",
                    "maximum": 2.0,
                    "unit": "seconds",
                    "regression": {
                        "max_ratio": 1.5,
                        "absolute_tolerance": 0.05,
                    },
                }
            ],
        }

    def write_case(self, directory: Path, values: list[float]) -> tuple[Path, list[Path]]:
        config_path = directory / "thresholds.json"
        config_path.write_text(json.dumps(self.config()), encoding="utf-8")
        sample_paths = []
        for index, value in enumerate(values):
            path = directory / f"sample-{index:02}.json"
            path.write_text(json.dumps(self.sample(value)), encoding="utf-8")
            sample_paths.append(path)
        return config_path, sample_paths

    def test_nearest_rank_p95_ignores_one_outlier_in_twenty_samples(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            config, samples = self.write_case(
                Path(temporary), [0.2] * 19 + [10.0]
            )
            result = evaluate_module.evaluate(config, samples)
            self.assertTrue(result["passed"])
            self.assertEqual(result["metrics"][0]["observed"], 0.2)

    def test_absolute_limit_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            config, samples = self.write_case(Path(temporary), [2.1] * 20)
            result = evaluate_module.evaluate(config, samples)
            self.assertFalse(result["passed"])

    def test_rejects_mixed_runner_images(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            config, samples = self.write_case(directory, [0.2] * 20)
            changed = json.loads(samples[-1].read_text(encoding="utf-8"))
            changed["runner"]["image_version"] = "different"
            samples[-1].write_text(json.dumps(changed), encoding="utf-8")
            with self.assertRaises(evaluate_module.EvaluationError):
                evaluate_module.evaluate(config, samples)

    def test_versioned_baseline_enforces_noise_aware_limit(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            config, baseline_samples = self.write_case(directory, [0.2] * 20)
            baseline = evaluate_module.evaluate(config, baseline_samples)
            baseline_path = directory / "baseline.json"
            baseline_path.write_text(json.dumps(baseline), encoding="utf-8")
            _, current_samples = self.write_case(directory, [0.31] * 20)
            result = evaluate_module.evaluate(config, current_samples, baseline_path)
            self.assertFalse(result["passed"])
            self.assertAlmostEqual(
                result["metrics"][0]["effective_maximum"], 0.3
            )


if __name__ == "__main__":
    unittest.main()
