from __future__ import annotations

import asyncio
import os
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))
sys.path.insert(0, str(ROOT / "packages/heiwa_protocol"))
sys.path.insert(0, str(ROOT / "packages/heiwa_identity"))
sys.path.insert(0, str(ROOT / "apps"))


async def _bind_runtime(spine, executor) -> None:
    from heiwa_protocol.protocol import Subject

    await spine.listen(Subject.CORE_REQUEST, spine.handle_request)
    await spine.listen(Subject.NODE_HEARTBEAT, spine.handle_heartbeat)
    await spine.listen(Subject.TASK_APPROVAL_DECISION, spine.handle_approval_decision)
    await executor.listen(Subject.TASK_EXEC, executor._handle_exec)
    await executor.listen(Subject.TASK_EXEC_REQUEST_CODE, executor._handle_exec)
    await executor.listen(Subject.TASK_EXEC_REQUEST_RESEARCH, executor._handle_exec)


def main() -> int:
    with tempfile.TemporaryDirectory() as tmpdir:
        os.environ["HEIWA_STATE_BACKEND"] = "compatibility_sqlite"
        os.environ["DATABASE_PATH"] = str(Path(tmpdir) / "hub.db")
        os.environ["HEIWA_AUTH_TOKEN"] = "test-heiwa-token"
        os.environ["HEIWA_EXECUTOR_RUNTIME"] = "railway"
        os.environ["HEIWA_MASTER_KEY"] = "test-heiwa-master-key-32-bytes!!"
        os.environ["HEIWA_AUTO_APPROVE"] = "cli"
        os.environ["HEIWA_APPROVAL_TIMEOUT_SEC"] = "60"

        from fastapi.testclient import TestClient

        from apps.heiwa_hub import mcp_server
        from apps.heiwa_hub.agents.executor import ExecutorAgent
        from apps.heiwa_hub.agents.spine import SpineAgent
        from apps.heiwa_hub.cognition.approval import get_approval_registry
        from heiwa_protocol.routing import BROKER_ENVELOPE_VERSION, BrokerRouteResult

        auth_token = os.getenv("HEIWA_AUTH_TOKEN", "") or "test-heiwa-token"
        mcp_server.db.init_db()
        mcp_server.TASK_SNAPSHOTS.clear()

        approvals = get_approval_registry()
        approvals._states.clear()  # type: ignore[attr-defined]
        approvals._payloads.clear()  # type: ignore[attr-defined]

        spine = SpineAgent()
        executor = ExecutorAgent()

        async def _fake_execute(route, instruction: str):
            return 0, "DEPLOY_OK"

        executor.gateway.execute = _fake_execute  # type: ignore[method-assign]

        def _fake_enrich(request):
            return BrokerRouteResult(
                request_id=request.request_id,
                task_id=request.task_id,
                envelope_version=BROKER_ENVELOPE_VERSION,
                raw_text=request.raw_text,
                source_surface=request.source_surface,
                intent_class="deploy",
                risk_level="critical",
                privacy_level="local",
                compute_class=3,
                assigned_worker="",
                target_tool="heiwa_ops",
                target_model="",
                target_runtime="railway",
                target_tier="tier3_orchestrator",
                requires_approval=True,
                rationale="Critical deploy must hold for operator approval.",
                confidence=0.99,
                normalization={
                    "intent_class": "deploy",
                    "risk_level": "critical",
                    "requires_approval": True,
                    "preferred_runtime": "railway",
                    "preferred_tool": "heiwa_ops",
                    "preferred_tier": "tier3_orchestrator",
                    "normalized_instruction": request.raw_text,
                    "assumptions": [],
                    "missing_details": [],
                    "confidence": 0.99,
                    "underspecified": False,
                },
            )

        mcp_server.enrichment.enrich = _fake_enrich  # type: ignore[method-assign]
        asyncio.run(_bind_runtime(spine, executor))

        failures: list[str] = []
        headers = {"Authorization": f"Bearer {auth_token}"}

        with TestClient(mcp_server.app) as client:
            response = client.post(
                "/tasks",
                json={
                    "raw_text": "deploy the hub to production",
                    "sender_id": "web-user",
                    "source_surface": "web",
                },
                headers=headers,
            )
            if response.status_code != 200:
                failures.append(f"/tasks should return 200, got {response.status_code}: {response.text}")
            else:
                payload = response.json()
                task_id = payload.get("task_id", "")
                route = payload.get("route", {})
                if not task_id:
                    failures.append("/tasks did not return a task_id")
                if not route.get("requires_approval"):
                    failures.append(f"/tasks should require approval, got route={route}")

                if task_id:
                    awaiting = None
                    with client.websocket_connect(f"/ws/tasks/{task_id}?token={auth_token}") as ws:
                        for _ in range(10):
                            event = ws.receive_json()
                            status = str(event.get("run_status") or event.get("status") or "")
                            if status == "AWAITING_APPROVAL":
                                awaiting = event
                                break
                    if awaiting is None:
                        failures.append(f"/ws/tasks/{task_id} did not emit AWAITING_APPROVAL")

                    snapshot = client.get(f"/tasks/{task_id}", headers=headers)
                    if snapshot.status_code != 200:
                        failures.append(f"/tasks/{task_id} should return 200, got {snapshot.status_code}")
                    else:
                        body = snapshot.json()
                        if str(body.get("status") or "") != "AWAITING_APPROVAL":
                            failures.append(f"/tasks/{task_id} should be awaiting approval: {body}")

                    approvals_resp = client.get("/approvals", headers=headers)
                    if approvals_resp.status_code != 200:
                        failures.append(f"/approvals should return 200, got {approvals_resp.status_code}")
                    else:
                        pending = approvals_resp.json().get("approvals", [])
                        if not any(item.get("task_id") == task_id for item in pending):
                            failures.append(f"/approvals missing pending task {task_id}: {pending}")

                    approve_resp = client.post(
                        f"/tasks/{task_id}/approve",
                        json={"actor": "devon", "reason": "ship it"},
                        headers=headers,
                    )
                    if approve_resp.status_code != 200:
                        failures.append(f"/tasks/{task_id}/approve should return 200, got {approve_resp.status_code}")

                    terminal = None
                    deadline = time.time() + 5.0
                    while time.time() < deadline:
                        polled = client.get(f"/tasks/{task_id}", headers=headers)
                        if polled.status_code == 200:
                            body = polled.json()
                            status = str(body.get("run_status") or body.get("status") or "")
                            if status == "PASS":
                                terminal = body
                                break
                        time.sleep(0.1)

                    if terminal is None:
                        failures.append(f"/tasks/{task_id} did not resume to PASS after approval")
                    elif "DEPLOY_OK" not in str(terminal.get("summary") or ""):
                        failures.append(f"terminal summary missing DEPLOY_OK: {terminal}")

        if failures:
            print("Approval gate e2e test FAILED")
            for failure in failures:
                print(f" - {failure}")
            return 1

        print("Approval gate e2e test PASSED")
        return 0


if __name__ == "__main__":
    raise SystemExit(main())
