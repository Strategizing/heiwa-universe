from __future__ import annotations

import asyncio
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

        from fastapi.testclient import TestClient

        from apps.heiwa_hub import mcp_server
        from apps.heiwa_hub.agents.executor import ExecutorAgent
        from apps.heiwa_hub.agents.spine import SpineAgent

        auth_token = os.getenv("HEIWA_AUTH_TOKEN", "") or "test-heiwa-token"
        mcp_server.db.init_db()
        mcp_server.TASK_SNAPSHOTS.clear()

        spine = SpineAgent()
        executor = ExecutorAgent()

        async def _fake_execute(route, instruction: str):
            return 0, "OK"

        executor.gateway.execute = _fake_execute  # type: ignore[method-assign]
        asyncio.run(_bind_runtime(spine, executor))

        failures: list[str] = []
        headers = {"Authorization": f"Bearer {auth_token}"}

        with TestClient(mcp_server.app) as client:
            response = client.post(
                "/tasks",
                json={
                    "raw_text": "reply with exactly OK",
                    "sender_id": "test-cli",
                    "source_surface": "cli",
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
                if route.get("target_tool") != "heiwa_claw":
                    failures.append(f"/tasks chose unexpected target_tool: {route}")

                if task_id:
                    terminal = None
                    seen_payloads: list[dict] = []
                    with client.websocket_connect(f"/ws/tasks/{task_id}?token={auth_token}") as ws:
                        for _ in range(10):
                            event = ws.receive_json()
                            seen_payloads.append(event)
                            status = str(event.get("run_status") or event.get("status") or "")
                            if status in {"PASS", "FAIL", "DELIVERED", "BLOCKED_AUTH", "BLOCKED_NO_CONTENT"}:
                                terminal = event
                                break

                    if terminal is None:
                        failures.append(f"/ws/tasks/{task_id} did not emit a terminal payload: {seen_payloads}")
                    else:
                        terminal_status = str(terminal.get("run_status") or terminal.get("status") or "")
                        if terminal_status != "PASS":
                            failures.append(f"terminal task status should be PASS, got {terminal_status}: {terminal}")
                        if "OK" not in str(terminal.get("summary") or ""):
                            failures.append(f"terminal payload should include executor summary 'OK': {terminal}")

                    task_snapshot = client.get(f"/tasks/{task_id}", headers=headers)
                    if task_snapshot.status_code != 200:
                        failures.append(f"/tasks/{task_id} should return 200, got {task_snapshot.status_code}")
                    else:
                        snapshot = task_snapshot.json()
                        if str(snapshot.get("run_status") or snapshot.get("status")) != "PASS":
                            failures.append(f"/tasks/{task_id} should persist PASS snapshot: {snapshot}")
                        if "OK" not in str(snapshot.get("summary") or ""):
                            failures.append(f"/tasks/{task_id} snapshot missing summary: {snapshot}")

        if failures:
            print("Task ingress e2e test FAILED")
            for failure in failures:
                print(f" - {failure}")
            return 1

        print("Task ingress e2e test PASSED")
        return 0


if __name__ == "__main__":
    raise SystemExit(main())
