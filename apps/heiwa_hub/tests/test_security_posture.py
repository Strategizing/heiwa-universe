from __future__ import annotations

import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))
sys.path.insert(0, str(ROOT / "packages/heiwa_protocol"))
sys.path.insert(0, str(ROOT / "packages/heiwa_identity"))
sys.path.insert(0, str(ROOT / "apps"))

from heiwa_sdk.security import redact_text
from heiwa_sdk.vault import InstanceVault


def main() -> int:
    failures: list[str] = []

    original_master_key = os.environ.pop("HEIWA_MASTER_KEY", None)
    try:
        try:
            InstanceVault()
            failures.append("InstanceVault should fail closed when HEIWA_MASTER_KEY is missing")
        except ValueError:
            pass
    finally:
        if original_master_key is not None:
            os.environ["HEIWA_MASTER_KEY"] = original_master_key

    redacted = redact_text("Authorization: Bearer abc123 NATS=nats://user:secret@example.com")
    if "abc123" in redacted or "secret" in redacted:
        failures.append("redaction failed to scrub transport credentials")

    original_state_backend = os.environ.get("HEIWA_STATE_BACKEND")
    original_stdb_identity = os.environ.pop("STDB_IDENTITY", None)
    os.environ["HEIWA_STATE_BACKEND"] = "spacetimedb"
    try:
        from heiwa_sdk.db import Database
        try:
            Database()
            failures.append("Database should fail closed when STDB backend is selected without STDB_IDENTITY")
        except ValueError:
            pass
    finally:
        if original_state_backend is not None:
            os.environ["HEIWA_STATE_BACKEND"] = original_state_backend
        else:
            os.environ.pop("HEIWA_STATE_BACKEND", None)
        if original_stdb_identity is not None:
            os.environ["STDB_IDENTITY"] = original_stdb_identity

    if failures:
        print("Security posture test FAILED")
        for failure in failures:
            print(f" - {failure}")
        return 1

    print("Security posture test PASSED")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
