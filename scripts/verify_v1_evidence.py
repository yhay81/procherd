#!/usr/bin/env python3
"""Validate the reviewable evidence required before an OSS CLI v1+ release."""

from __future__ import annotations

import argparse
import ipaddress
import json
import re
import sys
from datetime import date, timedelta
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

SCHEMA_VERSION = "oss-v1-evidence/v1"
PROJECT = "procherd"
GATE_NAMES = {
    "product_compatibility",
    "correctness_security",
    "performance_bounds",
    "delivery_maintenance",
    "adoption",
}
GATE_CRITERIA = {
    "product_compatibility": {
        "contract-compatibility",
        "golden-stores",
        "interruption-recovery",
        "explicit-platform-limitations",
    },
    "correctness_security": {
        "lifecycle-stress",
        "lease-stress",
        "store-adversarial-corpus",
        "log-pressure-accounting",
        "vulnerability-audit",
    },
    "performance_bounds": {
        "startup-overhead",
        "store-operation-latency",
        "supervisor-memory",
        "configured-bounds",
        "benchmark-reproducibility",
    },
    "delivery_maintenance": {
        "release-supply-chain",
        "security-response-slo",
    },
}
REVIEW_AREAS = {
    "store-permissions",
    "owner-tokens",
    "process-groups",
    "windows-job-objects",
    "probes",
    "leases",
    "path-handling",
    "symlinks",
    "recovery-races",
    "logs",
    "deletion-boundaries",
}
CI_TRACKS = {"lifecycle-stress", "linux", "macos", "windows"}
STATUSES = {"pending", "satisfied"}
CONTINUITY_MODES = {"two-maintainer-drill", "single-maintainer-recovery"}
MAX_MANIFEST_BYTES = 1024 * 1024
DATE_RE = re.compile(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}$")
PRERELEASE_IDENTIFIER = (
    r"(?:0|[1-9][0-9]*|[0-9]*[A-Za-z-][0-9A-Za-z-]*)"
)
SEMVER_RE = re.compile(
    r"^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
    rf"(?:-{PRERELEASE_IDENTIFIER}(?:\.{PRERELEASE_IDENTIFIER})*)?"
    r"(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?$"
)


class EvidenceError(ValueError):
    """An evidence manifest is malformed."""


def _object(value: Any, path: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise EvidenceError(f"{path} must be an object")
    return value


def _exact_keys(value: dict[str, Any], expected: set[str], path: str) -> None:
    actual = set(value)
    missing = sorted(expected - actual)
    unknown = sorted(actual - expected)
    if missing:
        raise EvidenceError(f"{path} is missing fields: {', '.join(missing)}")
    if unknown:
        raise EvidenceError(f"{path} has unknown fields: {', '.join(unknown)}")


def _string(value: Any, path: str, *, allow_empty: bool = False) -> str:
    if not isinstance(value, str):
        raise EvidenceError(f"{path} must be a string")
    if not allow_empty and not value.strip():
        raise EvidenceError(f"{path} must not be empty")
    return value


def _nonnegative_integer(value: Any, path: str) -> int:
    if isinstance(value, bool) or not isinstance(value, int) or value < 0:
        raise EvidenceError(f"{path} must be a non-negative integer")
    return value


def _status(value: Any, path: str) -> str:
    status = _string(value, path)
    if status not in STATUSES:
        raise EvidenceError(f"{path} must be one of: {', '.join(sorted(STATUSES))}")
    return status


def _date(value: Any, path: str, *, nullable: bool = False) -> date | None:
    if value is None and nullable:
        return None
    text = _string(value, path)
    if not DATE_RE.fullmatch(text):
        raise EvidenceError(f"{path} must use canonical YYYY-MM-DD form")
    try:
        return date.fromisoformat(text)
    except ValueError as error:
        raise EvidenceError(f"{path} must be an ISO 8601 calendar date") from error


def _https_url(value: Any, path: str) -> str:
    text = _string(value, path)
    if text != text.strip() or any(character.isspace() for character in text):
        raise EvidenceError(f"{path} must not contain whitespace")
    if "\\" in text:
        raise EvidenceError(f"{path} must not contain backslashes")
    parsed = urlparse(text)
    hostname = parsed.hostname
    if (
        parsed.scheme != "https"
        or not parsed.netloc
        or hostname is None
        or parsed.username
        or parsed.password
    ):
        raise EvidenceError(f"{path} must be a public HTTPS URL without credentials")
    normalized_hostname = hostname.rstrip(".").casefold()
    if (
        "." not in normalized_hostname
        or normalized_hostname == "localhost"
        or normalized_hostname.endswith(".localhost")
        or normalized_hostname.endswith(".local")
    ):
        raise EvidenceError(f"{path} must use a public hostname")
    try:
        address = ipaddress.ip_address(normalized_hostname)
    except ValueError:
        pass
    else:
        if not address.is_global:
            raise EvidenceError(f"{path} must not use a private or reserved IP address")
    return text


def _reference_item(value: Any, path: str) -> dict[str, str]:
    item = _object(value, path)
    _exact_keys(item, {"label", "url"}, path)
    _string(item["label"], f"{path}.label")
    _https_url(item["url"], f"{path}.url")
    return item


def _reference_list(value: Any, path: str) -> list[dict[str, str]]:
    if not isinstance(value, list):
        raise EvidenceError(f"{path} must be an array")
    return [_reference_item(item, f"{path}[{index}]") for index, item in enumerate(value)]


def _optional_reference_item(value: Any, path: str) -> dict[str, str] | None:
    if value is None:
        return None
    return _reference_item(value, path)


def _gate_evidence(value: Any, path: str, expected: set[str]) -> None:
    if not isinstance(value, list):
        raise EvidenceError(f"{path} must be an array")
    criteria: list[str] = []
    for index, raw_item in enumerate(value):
        item_path = f"{path}[{index}]"
        item = _object(raw_item, item_path)
        _exact_keys(item, {"criterion", "url"}, item_path)
        criterion = _string(item["criterion"], f"{item_path}.criterion")
        _https_url(item["url"], f"{item_path}.url")
        criteria.append(criterion)
    if len(criteria) != len(set(criteria)):
        raise EvidenceError(f"{path} must not contain duplicate criteria")
    unknown = sorted(set(criteria) - expected)
    if unknown:
        raise EvidenceError(f"{path} has unknown criteria: {', '.join(unknown)}")


def _basic_gate(value: Any, path: str, expected: set[str]) -> None:
    gate = _object(value, path)
    _exact_keys(gate, {"status", "evidence"}, path)
    _status(gate["status"], f"{path}.status")
    _gate_evidence(gate["evidence"], f"{path}.evidence", expected)


def _review(value: Any, path: str) -> None:
    if value is None:
        return
    review = _object(value, path)
    _exact_keys(review, {"reviewer", "completed_on", "url"}, path)
    _string(review["reviewer"], f"{path}.reviewer")
    _date(review["completed_on"], f"{path}.completed_on")
    _https_url(review["url"], f"{path}.url")


def _correctness_gate(value: Any, path: str) -> None:
    gate = _object(value, path)
    _exact_keys(
        gate,
        {
            "status",
            "evidence",
            "independent_review",
            "open_critical",
            "open_high",
            "reviewed_areas",
        },
        path,
    )
    _status(gate["status"], f"{path}.status")
    _gate_evidence(
        gate["evidence"],
        f"{path}.evidence",
        GATE_CRITERIA["correctness_security"],
    )
    _review(gate["independent_review"], f"{path}.independent_review")
    _nonnegative_integer(gate["open_critical"], f"{path}.open_critical")
    _nonnegative_integer(gate["open_high"], f"{path}.open_high")
    if not isinstance(gate["reviewed_areas"], list):
        raise EvidenceError(f"{path}.reviewed_areas must be an array")
    areas = [
        _string(area, f"{path}.reviewed_areas[{index}]")
        for index, area in enumerate(gate["reviewed_areas"])
    ]
    if len(set(areas)) != len(areas):
        raise EvidenceError(f"{path}.reviewed_areas must not contain duplicates")
    unknown = sorted(set(areas) - REVIEW_AREAS)
    if unknown:
        raise EvidenceError(f"{path}.reviewed_areas has unknown values: {', '.join(unknown)}")


def _ci_window(value: Any, path: str) -> None:
    window = _object(value, path)
    _exact_keys(window, {"start", "end", "required_days", "tracks"}, path)
    _date(window["start"], f"{path}.start", nullable=True)
    _date(window["end"], f"{path}.end", nullable=True)
    required_days = _nonnegative_integer(window["required_days"], f"{path}.required_days")
    if required_days != 30:
        raise EvidenceError(f"{path}.required_days must remain 30")
    tracks = _object(window["tracks"], f"{path}.tracks")
    _exact_keys(tracks, CI_TRACKS, f"{path}.tracks")
    for track in sorted(CI_TRACKS):
        runs = tracks[track]
        run_path = f"{path}.tracks.{track}"
        if not isinstance(runs, list):
            raise EvidenceError(f"{run_path} must be an array")
        dates: list[date] = []
        for index, raw_run in enumerate(runs):
            item_path = f"{run_path}[{index}]"
            run = _object(raw_run, item_path)
            _exact_keys(run, {"date", "url"}, item_path)
            run_date = _date(run["date"], f"{item_path}.date")
            if run_date is not None:
                dates.append(run_date)
            _https_url(run["url"], f"{item_path}.url")
        if len(dates) != len(set(dates)):
            raise EvidenceError(f"{run_path} must not contain duplicate dates")


def _continuity(value: Any, path: str) -> None:
    continuity = _object(value, path)
    _exact_keys(continuity, {"status", "mode", "evidence"}, path)
    _status(continuity["status"], f"{path}.status")
    mode = continuity["mode"]
    if mode is not None:
        mode = _string(mode, f"{path}.mode")
        if mode not in CONTINUITY_MODES:
            raise EvidenceError(
                f"{path}.mode must be one of: {', '.join(sorted(CONTINUITY_MODES))}"
            )
    _reference_list(continuity["evidence"], f"{path}.evidence")


def _delivery_gate(value: Any, path: str) -> None:
    gate = _object(value, path)
    _exact_keys(gate, {"status", "evidence", "ci_window", "continuity"}, path)
    _status(gate["status"], f"{path}.status")
    _gate_evidence(
        gate["evidence"],
        f"{path}.evidence",
        GATE_CRITERIA["delivery_maintenance"],
    )
    _ci_window(gate["ci_window"], f"{path}.ci_window")
    _continuity(gate["continuity"], f"{path}.continuity")


def _adopter(value: Any, path: str) -> None:
    adopter = _object(value, path)
    _exact_keys(adopter, {"name", "workflow", "outcome", "first_use", "evidence"}, path)
    _string(adopter["name"], f"{path}.name")
    _string(adopter["workflow"], f"{path}.workflow")
    _string(adopter["outcome"], f"{path}.outcome")
    _date(adopter["first_use"], f"{path}.first_use")
    _https_url(adopter["evidence"], f"{path}.evidence")


def _repeat_adopter(value: Any, path: str) -> None:
    repeat = _object(value, path)
    _exact_keys(repeat, {"adopter", "repeat_use", "evidence"}, path)
    _string(repeat["adopter"], f"{path}.adopter")
    _date(repeat["repeat_use"], f"{path}.repeat_use")
    _https_url(repeat["evidence"], f"{path}.evidence")


def _adoption_gate(value: Any, path: str) -> None:
    gate = _object(value, path)
    _exact_keys(
        gate,
        {
            "status",
            "adopters",
            "repeat_adopters",
            "public_integration",
            "non_maintainer_contribution",
        },
        path,
    )
    _status(gate["status"], f"{path}.status")
    if not isinstance(gate["adopters"], list):
        raise EvidenceError(f"{path}.adopters must be an array")
    for index, adopter in enumerate(gate["adopters"]):
        _adopter(adopter, f"{path}.adopters[{index}]")
    if not isinstance(gate["repeat_adopters"], list):
        raise EvidenceError(f"{path}.repeat_adopters must be an array")
    for index, repeat in enumerate(gate["repeat_adopters"]):
        _repeat_adopter(repeat, f"{path}.repeat_adopters[{index}]")
    _optional_reference_item(gate["public_integration"], f"{path}.public_integration")
    _optional_reference_item(
        gate["non_maintainer_contribution"],
        f"{path}.non_maintainer_contribution",
    )


def validate_structure(manifest: Any) -> dict[str, Any]:
    """Return a structurally valid manifest or raise EvidenceError."""
    root = _object(manifest, "$")
    _exact_keys(root, {"schema_version", "project", "target_version", "as_of", "gates"}, "$")
    if _string(root["schema_version"], "$.schema_version") != SCHEMA_VERSION:
        raise EvidenceError(f"$.schema_version must be {SCHEMA_VERSION!r}")
    if _string(root["project"], "$.project") != PROJECT:
        raise EvidenceError(f"$.project must be {PROJECT!r}")
    target_version = _string(root["target_version"], "$.target_version")
    if not SEMVER_RE.fullmatch(target_version):
        raise EvidenceError("$.target_version must be a valid SemVer version")
    _date(root["as_of"], "$.as_of")

    gates = _object(root["gates"], "$.gates")
    _exact_keys(gates, GATE_NAMES, "$.gates")
    _basic_gate(
        gates["product_compatibility"],
        "$.gates.product_compatibility",
        GATE_CRITERIA["product_compatibility"],
    )
    _correctness_gate(gates["correctness_security"], "$.gates.correctness_security")
    _basic_gate(
        gates["performance_bounds"],
        "$.gates.performance_bounds",
        GATE_CRITERIA["performance_bounds"],
    )
    _delivery_gate(gates["delivery_maintenance"], "$.gates.delivery_maintenance")
    _adoption_gate(gates["adoption"], "$.gates.adoption")
    return root


def readiness_errors(
    manifest: dict[str, Any], release_version: str | None = None
) -> list[dict[str, str]]:
    """Derive v1 release readiness instead of trusting a single boolean."""
    errors: list[dict[str, str]] = []

    def fail(code: str, message: str) -> None:
        errors.append({"code": code, "message": message})

    target_version = manifest["target_version"]
    match = SEMVER_RE.fullmatch(target_version)
    if match is None or int(match.group(1)) < 1:
        fail("target-version", "target_version must identify a v1+ release")
    if release_version is not None:
        if not SEMVER_RE.fullmatch(release_version):
            fail("release-version-format", "release version is not valid SemVer")
        elif target_version != release_version:
            fail(
                "release-version",
                f"target_version {target_version!r} does not match release {release_version!r}",
            )

    gates = manifest["gates"]
    for gate_name in sorted(GATE_NAMES):
        gate = gates[gate_name]
        if gate["status"] != "satisfied":
            fail("gate-status", f"{gate_name} is not satisfied")
    for gate_name, expected_criteria in sorted(GATE_CRITERIA.items()):
        actual_criteria = {
            item["criterion"] for item in gates[gate_name]["evidence"]
        }
        missing_criteria = sorted(expected_criteria - actual_criteria)
        if missing_criteria:
            fail(
                "gate-evidence",
                f"{gate_name} is missing evidence for: {', '.join(missing_criteria)}",
            )

    correctness = gates["correctness_security"]
    if correctness["independent_review"] is None:
        fail("security-review", "independent security review evidence is missing")
    if set(correctness["reviewed_areas"]) != REVIEW_AREAS:
        missing = ", ".join(sorted(REVIEW_AREAS - set(correctness["reviewed_areas"])))
        fail("security-scope", f"independent security review is missing areas: {missing}")
    if correctness["open_critical"] != 0 or correctness["open_high"] != 0:
        fail("security-findings", "critical and high security findings must both be zero")

    delivery = gates["delivery_maintenance"]
    window = delivery["ci_window"]
    start = _date(window["start"], "$.gates.delivery_maintenance.ci_window.start", nullable=True)
    end = _date(window["end"], "$.gates.delivery_maintenance.ci_window.end", nullable=True)
    if start is None or end is None:
        fail("ci-window", "the 30-day CI window is incomplete")
    elif end < start:
        fail("ci-window", "the CI window ends before it starts")
    elif (end - start).days + 1 < window["required_days"]:
        fail("ci-window", "the CI window covers fewer than 30 consecutive calendar days")
    if end is not None and end != _date(manifest["as_of"], "$.as_of"):
        fail("ci-freshness", "the CI window must end on the manifest as_of date")
    expected_dates: set[date] = set()
    if start is not None and end is not None and end >= start:
        expected_dates = {
            start + timedelta(days=offset)
            for offset in range((end - start).days + 1)
        }
    for track in sorted(CI_TRACKS):
        actual_dates = {
            _date(run["date"], f"$.gates.delivery_maintenance.ci_window.{track}[]")
            for run in window["tracks"][track]
        }
        missing_dates = sorted(expected_dates - actual_dates)
        extra_dates = sorted(actual_dates - expected_dates)
        if missing_dates:
            fail(
                "ci-platform",
                f"{track} continuous evidence is missing dates: "
                + ", ".join(day.isoformat() for day in missing_dates),
            )
        if extra_dates:
            fail(
                "ci-platform",
                f"{track} continuous evidence falls outside the window: "
                + ", ".join(day.isoformat() for day in extra_dates),
            )

    continuity = delivery["continuity"]
    if continuity["status"] != "satisfied":
        fail("continuity-status", "maintainer continuity is not satisfied")
    if continuity["mode"] not in CONTINUITY_MODES:
        fail("continuity-mode", "maintainer continuity mode is missing")
    if not continuity["evidence"]:
        fail("continuity-evidence", "maintainer continuity evidence is missing")

    adoption = gates["adoption"]
    adopters = adoption["adopters"]
    adopter_names = [adopter["name"].strip().casefold() for adopter in adopters]
    unique_adopters = set(adopter_names)
    if len(adopters) < 3 or len(unique_adopters) < 3:
        fail("adopters", "at least three distinct independent adopters are required")
    if len(unique_adopters) != len(adopter_names):
        fail("adopter-duplicates", "adopter names must be unique")

    first_use_by_name = {
        adopter["name"].strip().casefold(): _date(
            adopter["first_use"], "$.gates.adoption.adopters[].first_use"
        )
        for adopter in adopters
    }
    repeats = adoption["repeat_adopters"]
    repeated_names: set[str] = set()
    for repeat in repeats:
        name = repeat["adopter"].strip().casefold()
        if name in repeated_names:
            fail("repeat-duplicates", f"repeat evidence for {repeat['adopter']!r} is duplicated")
        repeated_names.add(name)
        first_use = first_use_by_name.get(name)
        repeat_use = _date(
            repeat["repeat_use"], "$.gates.adoption.repeat_adopters[].repeat_use"
        )
        if first_use is None:
            fail("repeat-adopter", f"repeat adopter {repeat['adopter']!r} is not an adopter")
        elif (repeat_use - first_use).days < 30:
            fail(
                "repeat-window",
                f"repeat use for {repeat['adopter']!r} is separated by fewer than 30 days",
            )
    if len(repeated_names & unique_adopters) < 2:
        fail("repeat-adopters", "at least two distinct adopters need 30-day repeat use")
    if adoption["public_integration"] is None:
        fail("public-integration", "required public integration evidence is missing")
    if adoption["non_maintainer_contribution"] is None:
        fail("external-contribution", "resolved non-maintainer contribution is missing")

    as_of = _date(manifest["as_of"], "$.as_of")
    dated_evidence: list[date] = []
    if end is not None:
        dated_evidence.append(end)
    review = correctness["independent_review"]
    if review is not None:
        completed_on = _date(
            review["completed_on"],
            "$.gates.correctness_security.independent_review.completed_on",
        )
        if completed_on is not None:
            dated_evidence.append(completed_on)
    dated_evidence.extend(
        evidence_date
        for evidence_date in first_use_by_name.values()
        if evidence_date is not None
    )
    dated_evidence.extend(
        repeat_date
        for repeat in repeats
        if (
            repeat_date := _date(
                repeat["repeat_use"],
                "$.gates.adoption.repeat_adopters[].repeat_use",
            )
        )
        is not None
    )
    if any(evidence_date > as_of for evidence_date in dated_evidence):
        fail("as-of", "as_of predates one or more evidence dates")

    return errors


def _unique_object(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise EvidenceError(f"JSON object contains duplicate field {key!r}")
        result[key] = value
    return result


def _load_manifest(path: Path) -> Any:
    try:
        raw = path.read_bytes()
    except OSError as error:
        raise EvidenceError(f"cannot read {path}: {error}") from error
    if len(raw) > MAX_MANIFEST_BYTES:
        raise EvidenceError(
            f"{path} exceeds the {MAX_MANIFEST_BYTES}-byte manifest limit"
        )
    try:
        text = raw.decode("utf-8")
        return json.loads(text, object_pairs_hook=_unique_object)
    except UnicodeDecodeError as error:
        raise EvidenceError(f"{path} is not valid UTF-8") from error
    except json.JSONDecodeError as error:
        raise EvidenceError(
            f"{path} is not valid JSON: line {error.lineno}, column {error.colno}"
        ) from error
    except RecursionError as error:
        raise EvidenceError(f"{path} exceeds the supported JSON nesting depth") from error


def _parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manifest", type=Path)
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument("--check-structure", action="store_true")
    mode.add_argument("--require-ready", action="store_true")
    parser.add_argument(
        "--release-version",
        help="exact Cargo package version; required with --require-ready",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    args = _parser().parse_args(argv)
    if args.require_ready and not args.release_version:
        raise SystemExit("--release-version is required with --require-ready")
    if args.check_structure and args.release_version:
        raise SystemExit("--release-version is only valid with --require-ready")

    try:
        manifest = validate_structure(_load_manifest(args.manifest))
    except EvidenceError as error:
        print(
            json.dumps(
                {
                    "schema_version": "oss-v1-evidence-report/v1",
                    "structure_valid": False,
                    "release_ready": False,
                    "errors": [{"code": "structure", "message": str(error)}],
                },
                sort_keys=True,
            )
        )
        return 2

    if args.check_structure:
        print(
            json.dumps(
                {
                    "schema_version": "oss-v1-evidence-report/v1",
                    "structure_valid": True,
                    "release_ready": None,
                    "errors": [],
                },
                sort_keys=True,
            )
        )
        return 0

    errors = readiness_errors(manifest, release_version=args.release_version)
    report = {
        "schema_version": "oss-v1-evidence-report/v1",
        "structure_valid": True,
        "release_ready": not errors,
        "errors": errors,
    }
    print(json.dumps(report, sort_keys=True))
    if args.require_ready and errors:
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
