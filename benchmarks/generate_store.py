#!/usr/bin/env python3
"""Generate a deterministic terminal ProcHerd store for benchmarks."""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import pathlib


CROCKFORD_BASE32 = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
FIXED_TIMESTAMP_MS = 1_720_000_000_000


def positive_integer(value: str) -> int:
    parsed = int(value)
    if parsed < 1:
        raise argparse.ArgumentTypeError("runs must be at least 1")
    return parsed


def encode_ulid(timestamp_ms: int, randomness: int) -> str:
    value = (timestamp_ms << 80) | randomness
    encoded = ["0"] * 26
    for index in range(25, -1, -1):
        encoded[index] = CROCKFORD_BASE32[value & 31]
        value >>= 5
    return "".join(encoded)


def update_digest(digest: hashlib._Hash, relative: str, content: bytes) -> None:
    relative_bytes = relative.encode()
    digest.update(len(relative_bytes).to_bytes(8, "little"))
    digest.update(relative_bytes)
    digest.update(len(content).to_bytes(8, "little"))
    digest.update(content)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--runs", required=True, type=positive_integer)
    parser.add_argument("--output", required=True, type=pathlib.Path)
    args = parser.parse_args()

    if args.output.exists():
        parser.error("output must not already exist")

    repository = pathlib.Path(__file__).resolve().parent.parent
    fixture = (
        repository
        / "tests"
        / "fixtures"
        / "contracts"
        / "v0.1"
        / "run_01J00000000000000000000000"
    )
    state_template = json.loads((fixture / "state.json").read_text(encoding="utf-8"))
    logs = (fixture / "logs.ndjson").read_bytes()
    owner_token = (fixture / "owner.token").read_bytes()

    args.output.mkdir(parents=True, mode=0o700)
    digest = hashlib.sha256()
    run_ids: list[str] = []
    for index in range(args.runs):
        run_id = f"run_{encode_ulid(FIXED_TIMESTAMP_MS, index)}"
        run_ids.append(run_id)
        run_dir = args.output / run_id
        run_dir.mkdir(mode=0o700)

        state = copy.deepcopy(state_template)
        state["run_id"] = run_id
        state["created_at_ms"] = FIXED_TIMESTAMP_MS + index * 2
        state["updated_at_ms"] = FIXED_TIMESTAMP_MS + index * 2 + 1
        state["exit"]["finished_at_ms"] = state["updated_at_ms"]
        state["cleanup"]["completed_at_ms"] = state["updated_at_ms"]
        state_bytes = (json.dumps(state, indent=2) + "\n").encode()

        files = {
            "logs.ndjson": logs,
            "owner.token": owner_token,
            "state.json": state_bytes,
            "supervisor.lock": b"",
        }
        for name, content in files.items():
            path = run_dir / name
            path.write_bytes(content)
            path.chmod(0o600)
            update_digest(digest, f"{run_id}/{name}", content)

    print(
        json.dumps(
            {
                "schema_version": "procherd.benchmark-store.v1",
                "runs": args.runs,
                "first_run_id": run_ids[0],
                "last_run_id": run_ids[-1],
                "content_sha256": digest.hexdigest(),
            },
            separators=(",", ":"),
        )
    )


if __name__ == "__main__":
    main()
