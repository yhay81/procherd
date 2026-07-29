from __future__ import annotations

import json
import sys
import tempfile
import unittest
from datetime import date, timedelta
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))

import verify_v1_evidence as verifier

REPOSITORY_URL = f"https://github.com/yhay81/{verifier.PROJECT}"


def evidence(label: str) -> dict[str, str]:
    return {"label": label, "url": f"{REPOSITORY_URL}/issues/{label}"}


def criterion_evidence(criterion: str) -> dict[str, str]:
    return {
        "criterion": criterion,
        "url": f"{REPOSITORY_URL}/issues/{criterion}",
    }


def ready_manifest() -> dict:
    manifest = json.loads((ROOT / ".github/v1-evidence.json").read_text(encoding="utf-8"))
    gates = manifest["gates"]
    for name in verifier.GATE_NAMES:
        gates[name]["status"] = "satisfied"
    for name, criteria in verifier.GATE_CRITERIA.items():
        gates[name]["evidence"] = [
            criterion_evidence(criterion) for criterion in sorted(criteria)
        ]

    security = gates["correctness_security"]
    security["independent_review"] = {
        "reviewer": "independent-reviewer",
        "completed_on": "2026-07-25",
        "url": f"{REPOSITORY_URL}/issues/security-review",
    }
    security["reviewed_areas"] = sorted(verifier.REVIEW_AREAS)

    delivery = gates["delivery_maintenance"]
    delivery["ci_window"]["start"] = "2026-06-30"
    delivery["ci_window"]["end"] = "2026-07-29"
    for track in verifier.CI_TRACKS:
        delivery["ci_window"]["tracks"][track] = [
            {
                "date": (date(2026, 6, 30) + timedelta(days=offset)).isoformat(),
                "url": (
                    f"{REPOSITORY_URL}/actions/runs/{100000 + offset}"
                ),
            }
            for offset in range(30)
        ]
    delivery["continuity"] = {
        "status": "satisfied",
        "mode": sorted(verifier.CONTINUITY_MODES)[0],
        "evidence": [evidence("continuity")],
    }

    adoption = gates["adoption"]
    adoption["adopters"] = [
        {
            "name": name,
            "workflow": f"{name} production workflow",
            "outcome": "Verification or a safe refusal improved the decision.",
            "first_use": first_use,
            "evidence": f"{REPOSITORY_URL}/discussions/{index}",
        }
        for index, (name, first_use) in enumerate(
            [
                ("Studio Alpha", "2026-06-01"),
                ("Team Beta", "2026-06-10"),
                ("Creator Gamma", "2026-07-01"),
            ],
            start=1,
        )
    ]
    adoption["repeat_adopters"] = [
        {
            "adopter": "Studio Alpha",
            "repeat_use": "2026-07-01",
            "evidence": f"{REPOSITORY_URL}/discussions/11",
        },
        {
            "adopter": "Team Beta",
            "repeat_use": "2026-07-10",
            "evidence": f"{REPOSITORY_URL}/discussions/12",
        },
    ]
    adoption["public_integration"] = evidence("public-integration")
    adoption["non_maintainer_contribution"] = evidence("external-contribution")
    return manifest


class StructureTests(unittest.TestCase):
    def test_checked_in_pending_manifest_is_structurally_valid(self) -> None:
        manifest = json.loads(
            (ROOT / ".github/v1-evidence.json").read_text(encoding="utf-8")
        )
        verifier.validate_structure(manifest)
        self.assertTrue(verifier.readiness_errors(manifest, "1.0.0"))

    def test_unknown_field_is_rejected(self) -> None:
        manifest = ready_manifest()
        manifest["release_ready"] = True
        with self.assertRaisesRegex(verifier.EvidenceError, "unknown fields"):
            verifier.validate_structure(manifest)

    def test_non_https_evidence_is_rejected(self) -> None:
        manifest = ready_manifest()
        manifest["gates"]["product_compatibility"]["evidence"][0]["url"] = (
            "http://example.test/evidence"
        )
        with self.assertRaisesRegex(verifier.EvidenceError, "public HTTPS URL"):
            verifier.validate_structure(manifest)

    def test_unknown_gate_criterion_is_rejected(self) -> None:
        manifest = ready_manifest()
        manifest["gates"]["performance_bounds"]["evidence"][0]["criterion"] = (
            "download-count"
        )
        with self.assertRaisesRegex(verifier.EvidenceError, "unknown criteria"):
            verifier.validate_structure(manifest)

    def test_duplicate_json_field_is_rejected_before_validation(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            manifest_path = Path(directory) / "evidence.json"
            manifest_path.write_text('{"schema_version":"first","schema_version":"second"}')
            with self.assertRaisesRegex(verifier.EvidenceError, "duplicate field"):
                verifier._load_manifest(manifest_path)


class ReadinessTests(unittest.TestCase):
    def test_complete_manifest_is_ready(self) -> None:
        manifest = verifier.validate_structure(ready_manifest())
        self.assertEqual(verifier.readiness_errors(manifest, "1.0.0"), [])

    def test_release_version_must_match(self) -> None:
        manifest = verifier.validate_structure(ready_manifest())
        codes = {
            error["code"] for error in verifier.readiness_errors(manifest, "1.0.1")
        }
        self.assertIn("release-version", codes)

    def test_ci_window_counts_consecutive_calendar_days(self) -> None:
        manifest = ready_manifest()
        manifest["gates"]["delivery_maintenance"]["ci_window"]["start"] = "2026-07-01"
        manifest = verifier.validate_structure(manifest)
        codes = {error["code"] for error in verifier.readiness_errors(manifest)}
        self.assertIn("ci-window", codes)
        self.assertIn("ci-platform", codes)

    def test_ci_window_requires_daily_evidence_for_each_platform(self) -> None:
        manifest = ready_manifest()
        manifest["gates"]["delivery_maintenance"]["ci_window"]["tracks"][
            "windows"
        ].pop()
        manifest = verifier.validate_structure(manifest)
        codes = {error["code"] for error in verifier.readiness_errors(manifest)}
        self.assertIn("ci-platform", codes)

    def test_duplicate_adopters_do_not_satisfy_the_gate(self) -> None:
        manifest = ready_manifest()
        manifest["gates"]["adoption"]["adopters"][2]["name"] = " studio alpha "
        manifest = verifier.validate_structure(manifest)
        codes = {error["code"] for error in verifier.readiness_errors(manifest)}
        self.assertIn("adopter-duplicates", codes)
        self.assertIn("adopters", codes)

    def test_repeat_use_must_be_at_least_30_days_later(self) -> None:
        manifest = ready_manifest()
        manifest["gates"]["adoption"]["repeat_adopters"][0]["repeat_use"] = "2026-06-30"
        manifest = verifier.validate_structure(manifest)
        codes = {error["code"] for error in verifier.readiness_errors(manifest)}
        self.assertIn("repeat-window", codes)

    def test_review_must_cover_every_required_area(self) -> None:
        manifest = ready_manifest()
        manifest["gates"]["correctness_security"]["reviewed_areas"].pop()
        manifest = verifier.validate_structure(manifest)
        codes = {error["code"] for error in verifier.readiness_errors(manifest)}
        self.assertIn("security-scope", codes)

    def test_open_high_finding_blocks_readiness(self) -> None:
        manifest = ready_manifest()
        manifest["gates"]["correctness_security"]["open_high"] = 1
        manifest = verifier.validate_structure(manifest)
        codes = {error["code"] for error in verifier.readiness_errors(manifest)}
        self.assertIn("security-findings", codes)

    def test_future_evidence_is_rejected(self) -> None:
        manifest = ready_manifest()
        manifest["gates"]["adoption"]["repeat_adopters"][0]["repeat_use"] = "2026-08-01"
        manifest = verifier.validate_structure(manifest)
        codes = {error["code"] for error in verifier.readiness_errors(manifest)}
        self.assertIn("as-of", codes)

    def test_ci_window_must_end_on_as_of_date(self) -> None:
        manifest = ready_manifest()
        manifest["as_of"] = "2026-07-30"
        manifest = verifier.validate_structure(manifest)
        codes = {error["code"] for error in verifier.readiness_errors(manifest)}
        self.assertIn("ci-freshness", codes)


if __name__ == "__main__":
    unittest.main()
