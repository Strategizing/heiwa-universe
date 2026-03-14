from __future__ import annotations

import os
import sys
import tempfile
import datetime
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))
sys.path.insert(0, str(ROOT / "packages/heiwa_protocol"))
sys.path.insert(0, str(ROOT / "packages/heiwa_identity"))
sys.path.insert(0, str(ROOT / "apps"))


def main() -> int:
    failures: list[str] = []

    with tempfile.TemporaryDirectory() as tmpdir:
        os.environ["HEIWA_STATE_BACKEND"] = "compatibility_sqlite"
        os.environ["DATABASE_PATH"] = str(Path(tmpdir) / "hub.db")

        from heiwa_sdk.db import Database
        from heiwa_sdk.state import HubStateService

        db = Database()
        db.init_db()
        now = datetime.datetime.now(datetime.timezone.utc)

        conn = db.get_connection()
        cursor = conn.cursor()
        db._exec(
            cursor,
            """
            INSERT INTO runs (
                run_id, proposal_id, started_at, ended_at, status, chain_result, model_id, tokens_total, cost
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "run-1",
                "proposal-1",
                (now - datetime.timedelta(minutes=1)).isoformat(),
                now.isoformat(),
                "PASS",
                "ok",
                "ollama/llama-4-scout:q4_k_m",
                42,
                0.0,
            ),
        )
        conn.commit()
        conn.close()

        state = HubStateService(db)
        runs = state.get_recent_runs(limit=1)
        status = state.get_public_status(minutes=60)

        if len(runs) != 1:
            failures.append(f"expected 1 recent run, got {len(runs)}")
        if status.get("state_backend") != "compatibility_sqlite":
            failures.append(f"expected compatibility_sqlite backend, got {status.get('state_backend')}")
        if status.get("active_models") != 1:
            failures.append(f"expected 1 active model, got {status.get('active_models')}")

    if failures:
        print("State service test FAILED")
        for failure in failures:
            print(f" - {failure}")
        return 1

    print("State service test PASSED")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
