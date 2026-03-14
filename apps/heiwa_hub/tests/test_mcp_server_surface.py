from __future__ import annotations

import datetime
import os
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))
sys.path.insert(0, str(ROOT / "packages/heiwa_protocol"))
sys.path.insert(0, str(ROOT / "packages/heiwa_identity"))
sys.path.insert(0, str(ROOT / "apps"))


def main() -> int:
    with tempfile.TemporaryDirectory() as tmpdir:
        os.environ["HEIWA_STATE_BACKEND"] = "compatibility_sqlite"
        os.environ["DATABASE_PATH"] = str(Path(tmpdir) / "hub.db")

        from fastapi.testclient import TestClient
        from apps.heiwa_hub import mcp_server

        mcp_server.db.init_db()
        now = datetime.datetime.now(datetime.timezone.utc)
        conn = mcp_server.db.get_connection()
        cursor = conn.cursor()
        mcp_server.db._exec(
            cursor,
            """
            INSERT INTO runs (
                run_id, proposal_id, started_at, ended_at, status, chain_result, model_id, tokens_total, cost
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "run-mcp",
                "proposal-mcp",
                (now - datetime.timedelta(minutes=1)).isoformat(),
                now.isoformat(),
                "PASS",
                "ok",
                "ollama/llama-4-scout:q4_k_m",
                11,
                0.0,
            ),
        )
        conn.commit()
        conn.close()

        client = TestClient(mcp_server.app)

        failures: list[str] = []
        if client.get("/health").status_code != 200:
            failures.append("/health should return 200")
        status_payload = client.get("/status").json()
        if status_payload.get("state_backend") != "compatibility_sqlite":
            failures.append(f"/status backend mismatch: {status_payload}")
        tools_payload = client.get("/tools").json()
        native_names = {tool["name"] for tool in tools_payload.get("native", [])}
        if "heiwa_resolve_route" not in native_names:
            failures.append("/tools is missing heiwa_resolve_route")
        if "heiwa_run_bench" not in native_names:
            failures.append("/tools is missing heiwa_run_bench")
        if "heiwa_get_cells_catalog" not in native_names:
            failures.append("/tools is missing heiwa_get_cells_catalog")
        call_payload = client.post("/call/heiwa_get_latest_tasks", json={"limit": 1}).json()
        text = call_payload["content"][0]["text"]
        if "run-mcp" not in text:
            failures.append("/call/heiwa_get_latest_tasks did not return seeded run")
        bench_payload = client.post("/call/heiwa_run_bench", json={}).json()
        if '"ok": true' not in bench_payload["content"][0]["text"]:
            failures.append("/call/heiwa_run_bench did not report success")
        cells_payload = client.post("/call/heiwa_get_cells_catalog", json={"prompt": "implement code refactor"}).json()
        if "codex-builder" not in cells_payload["content"][0]["text"]:
            failures.append("/call/heiwa_get_cells_catalog did not recommend codex-builder")

        if failures:
            print("MCP server surface test FAILED")
            for failure in failures:
                print(f" - {failure}")
            return 1

        print("MCP server surface test PASSED")
        return 0


if __name__ == "__main__":
    raise SystemExit(main())
