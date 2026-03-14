from __future__ import annotations

import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))
sys.path.insert(0, str(ROOT / "packages/heiwa_protocol"))
sys.path.insert(0, str(ROOT / "packages/heiwa_identity"))
sys.path.insert(0, str(ROOT / "apps"))


class FakeSpacetimeDB:
    def __init__(self, *args, **kwargs):
        self.calls: list[tuple[str, object]] = []

    def record_route_decision(self, route_data):
        self.calls.append(("record_route_decision", route_data))
        return True

    def record_run(self, run_data):
        self.calls.append(("record_run", run_data))
        return True

    def upsert_node_heartbeat(self, **kwargs):
        self.calls.append(("upsert_node_heartbeat", kwargs))
        return True

    def upsert_liveness_state(self, key, state, changed_at=None):
        self.calls.append(("upsert_liveness_state", {"key": key, "state": state, "changed_at": changed_at}))
        return True

    def get_liveness_state(self, key):
        self.calls.append(("get_liveness_state", key))
        return {"key": key, "last_state": "ONLINE", "last_changed_at": "2026-03-13T00:00:00+00:00"}

    def get_runs(self, proposal_id=None, limit=50):
        self.calls.append(("get_runs", {"proposal_id": proposal_id, "limit": limit}))
        return [{"run_id": "run-stdb", "proposal_id": "task-stdb", "model_id": "openai-codex/gpt-5.3-codex"}]

    def get_model_usage_summary(self, minutes=60):
        self.calls.append(("get_model_usage_summary", minutes))
        return [{"model_id": "openai-codex/gpt-5.3-codex", "request_count": 1, "total_tokens": 256, "total_cost": 0.12}]

    def list_nodes(self, status=None):
        self.calls.append(("list_nodes", status))
        rows = [
            {"node_id": "macbook@heiwa-node-a", "status": "ONLINE"},
            {"node_id": "pc@heiwa-node-b", "status": "SILENT"},
        ]
        if status:
            return [row for row in rows if row["status"] == status]
        return rows

    def get_node(self, node_id):
        self.calls.append(("get_node", node_id))
        return {"node_id": node_id, "status": "ONLINE"}


def main() -> int:
    failures: list[str] = []

    original_backend = os.environ.get("HEIWA_STATE_BACKEND")
    original_identity = os.environ.get("STDB_IDENTITY")
    os.environ["HEIWA_STATE_BACKEND"] = "spacetimedb"
    os.environ["STDB_IDENTITY"] = "heiwa_test_module"

    try:
        import heiwa_sdk.db as db_module
        from heiwa_protocol.routing import BrokerRouteResult
        from heiwa_sdk.state import HubStateService

        original_stdb_class = db_module.SpacetimeDB
        db_module.SpacetimeDB = FakeSpacetimeDB
        try:
            db = db_module.Database()
            route = BrokerRouteResult(
                request_id="req-stdb",
                task_id="task-stdb",
                envelope_version="2026-03-13",
                raw_text="build the state bridge",
                source_surface="cli",
                intent_class="build",
                risk_level="medium",
                privacy_level="local",
                compute_class=3,
                assigned_worker="broker",
                target_tool="heiwa_claw",
                target_model="openai-codex/gpt-5.3-codex",
                target_runtime="railway",
                target_tier="tier3_premium_build",
                requires_approval=False,
                rationale="premium build route",
                confidence=0.91,
            )

            if not db.record_route_decision(route.to_dict()):
                failures.append("record_route_decision should use the STDB adapter")
            if not db.record_run({"run_id": "run-stdb", "proposal_id": "task-stdb", "status": "PASS", "model_id": "openai-codex/gpt-5.3-codex"}):
                failures.append("record_run should use the STDB adapter")
            if not db.upsert_node_heartbeat("macbook@heiwa-node-a", meta={"cpu_pct": 10}):
                failures.append("upsert_node_heartbeat should use the STDB adapter")
            if not db.set_liveness_state("hub", "ONLINE"):
                failures.append("set_liveness_state should use the STDB adapter")

            if db.get_liveness_state("hub")["last_state"] != "ONLINE":
                failures.append("get_liveness_state should read from the STDB adapter")
            if len(db.get_runs(limit=1)) != 1:
                failures.append("get_runs should read from the STDB adapter")
            if len(db.get_model_usage_summary(minutes=60)) != 1:
                failures.append("get_model_usage_summary should read from the STDB adapter")
            if len(db.list_nodes(status="ONLINE")) != 1:
                failures.append("list_nodes(status='ONLINE') should filter through the STDB adapter")

            state = HubStateService(db)
            public_status = state.get_public_status(minutes=60)
            if public_status.get("live_nodes") != 1:
                failures.append(f"expected 1 live node in public status, got {public_status.get('live_nodes')}")
            if public_status.get("active_models") != 1:
                failures.append(f"expected 1 active model in public status, got {public_status.get('active_models')}")

            observed = {name for name, _ in db.stdb.calls}
            expected = {
                "record_route_decision",
                "record_run",
                "upsert_node_heartbeat",
                "upsert_liveness_state",
                "get_liveness_state",
                "get_runs",
                "get_model_usage_summary",
                "list_nodes",
            }
            missing = sorted(expected - observed)
            if missing:
                failures.append(f"missing STDB calls: {', '.join(missing)}")
        finally:
            db_module.SpacetimeDB = original_stdb_class
    finally:
        if original_backend is not None:
            os.environ["HEIWA_STATE_BACKEND"] = original_backend
        else:
            os.environ.pop("HEIWA_STATE_BACKEND", None)
        if original_identity is not None:
            os.environ["STDB_IDENTITY"] = original_identity
        else:
            os.environ.pop("STDB_IDENTITY", None)

    if failures:
        print("STDB native state test FAILED")
        for failure in failures:
            print(f" - {failure}")
        return 1

    print("STDB native state test PASSED")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
