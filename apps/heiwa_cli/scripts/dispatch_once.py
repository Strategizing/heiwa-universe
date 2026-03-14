"""
One-shot task dispatcher.

Sends task to Railway hub via HTTP POST. If hub is unreachable,
falls back to direct local execution via BrokerEnrichmentService
+ HeiwaClawGateway.
"""

import asyncio
import json
import logging
import os
import sys
import uuid
import time
from pathlib import Path

# Ensure project root is on sys.path
ROOT = Path(__file__).resolve().parents[3]
for p in ["packages/heiwa_sdk", "packages/heiwa_protocol", "packages/heiwa_identity", "apps"]:
    p_path = str(ROOT / p)
    if p_path not in sys.path:
        sys.path.insert(0, p_path)

from heiwa_protocol.routing import BrokerRouteRequest
from heiwa_sdk.config import hub_url_candidates, settings
from heiwa_sdk.heiwaclaw import HeiwaClawGateway
from heiwa_sdk.operator_surface import maybe_fast_path_turn
from heiwa_hub.cognition.enrichment import BrokerEnrichmentService


def _render_direct_output(output: str) -> str:
    text = str(output or "").strip()
    if not text:
        return ""
    start = text.find("{")
    end = text.rfind("}")
    if start != -1 and end > start:
        try:
            payload = json.loads(text[start : end + 1])
        except json.JSONDecodeError:
            return text
        payloads = payload.get("payloads")
        if isinstance(payloads, list):
            rendered = []
            for item in payloads:
                if isinstance(item, dict):
                    rendered_text = str(item.get("text") or "").strip()
                    if rendered_text:
                        rendered.append(rendered_text)
            if rendered:
                return "\n\n".join(rendered)
    return text


async def _direct_execute(prompt: str, node_id: str, task_id: str, token: str) -> int:
    """Fallback: enrich and execute locally without the Railway hub."""
    request = BrokerRouteRequest(
        request_id=f"cli-request-{task_id}",
        task_id=task_id,
        raw_text=prompt,
        sender_id=node_id,
        source_surface="cli",
        auth_validated=bool(token),
    )
    enrichment = BrokerEnrichmentService()
    route = enrichment.enrich(request)
    gateway = HeiwaClawGateway(ROOT)
    dispatch = gateway.resolve(route)

    print(
        f"[HEIWA] Direct route: {route.intent_class} -> "
        f"{dispatch.provider or dispatch.adapter_tool} via {dispatch.adapter_tool}"
    )
    print(f"[HEIWA] Rationale: {route.rationale}")
    exit_code, output = await gateway.execute(route, prompt)
    rendered = _render_direct_output(output)
    if rendered:
        print(rendered)
    return exit_code


async def _submit_to_hub(prompt: str, node_id: str, task_id: str, hub_url: str, token: str) -> dict | None:
    """POST task to hub. Returns the response dict on success, None on failure."""
    headers = {"Content-Type": "application/json", "Authorization": f"Bearer {token}"}
    body = json.dumps({
        "raw_text": prompt,
        "sender_id": node_id,
        "source_surface": "cli",
        "task_id": task_id,
    }).encode()

    try:
        import httpx
    except ImportError:
        import urllib.request
        import urllib.error
        req = urllib.request.Request(
            f"{hub_url}/tasks", data=body, headers=headers, method="POST",
        )
        try:
            with urllib.request.urlopen(req, timeout=10) as resp:
                return json.loads(resp.read().decode())
        except (urllib.error.URLError, urllib.error.HTTPError, OSError):
            return None

    async with httpx.AsyncClient(timeout=10.0) as client:
        resp = await client.post(f"{hub_url}/tasks", content=body, headers=headers)
        if resp.status_code == 200:
            return resp.json()
        print(f"[HEIWA] Hub returned HTTP {resp.status_code}: {resp.text[:200]}")
        return None


async def _stream_task_result(hub_url: str, task_id: str, token: str = "", timeout: float = 120.0) -> int:
    """Connect to WS /ws/tasks/{task_id}?token=... and stream events until terminal status."""
    ws_url = hub_url.replace("https://", "wss://").replace("http://", "ws://")
    ws_url = f"{ws_url}/ws/tasks/{task_id}?token={token}"

    try:
        import websockets
    except ImportError:
        print("[HEIWA] websockets not installed — cannot stream results from hub.")
        return 0

    try:
        async with websockets.connect(ws_url, open_timeout=10) as ws:
            deadline = time.time() + timeout
            while time.time() < deadline:
                try:
                    raw = await asyncio.wait_for(ws.recv(), timeout=35.0)
                except asyncio.TimeoutError:
                    continue
                event = json.loads(raw)
                evt_type = event.get("type", "")
                status = event.get("status", "")

                if evt_type == "heartbeat":
                    continue

                if status in {"ACKNOWLEDGED", "DISPATCHED_PLAN", "DISPATCHED_FALLBACK"}:
                    print(f"[HEIWA] {status}: {event.get('message', '')}")
                elif status in {"DELIVERED", "PASS"}:
                    summary = event.get("summary", "")
                    rendered = _render_direct_output(summary) if summary else ""
                    if rendered:
                        print(rendered)
                    return 0
                elif status in {"FAIL", "BLOCKED_AUTH", "BLOCKED_NO_CONTENT"}:
                    print(f"[HEIWA] {status}: {event.get('message', event.get('summary', ''))}")
                    return 1
                else:
                    content = event.get("content") or event.get("message") or ""
                    if content:
                        print(f"[HEIWA] {content}")
    except Exception as e:
        logging.getLogger("dispatch").debug("WS stream error: %s", e)
    return await _poll_task_result(hub_url, task_id, token=token, timeout=timeout)


async def _poll_task_result(hub_url: str, task_id: str, token: str = "", timeout: float = 120.0) -> int:
    """Fallback path when websocket streaming is unavailable."""
    headers = {"Authorization": f"Bearer {token}"} if token else {}
    deadline = time.time() + timeout

    async def _httpx_poll() -> tuple[int | None, dict | None]:
        import httpx
        async with httpx.AsyncClient(timeout=10.0) as client:
            resp = await client.get(f"{hub_url}/tasks/{task_id}", headers=headers)
            return resp.status_code, resp.json() if resp.status_code == 200 else None

    def _urllib_poll() -> tuple[int | None, dict | None]:
        import urllib.request
        import urllib.error
        req = urllib.request.Request(f"{hub_url}/tasks/{task_id}", headers=headers, method="GET")
        try:
            with urllib.request.urlopen(req, timeout=10) as resp:
                return resp.status, json.loads(resp.read().decode())
        except urllib.error.HTTPError as exc:
            return exc.code, None
        except OSError:
            return None, None

    while time.time() < deadline:
        try:
            try:
                status_code, payload = await _httpx_poll()
            except ImportError:
                status_code, payload = _urllib_poll()
        except Exception:
            status_code, payload = None, None

        if status_code == 200 and isinstance(payload, dict):
            status = str(payload.get("status") or payload.get("run_status") or "").upper()
            summary = str(payload.get("summary") or payload.get("result") or payload.get("content") or "").strip()
            if status in {"PASS", "SUCCESS", "COMPLETED"}:
                if summary:
                    print(_render_direct_output(summary))
                return 0
            if status in {"FAIL", "ERROR", "BLOCKED_AUTH", "BLOCKED_NO_CONTENT"}:
                print(f"[HEIWA] {status}: {summary or 'task failed'}")
                return 1
        await asyncio.sleep(2.0)

    print("[HEIWA] Timed out waiting for task result from hub.")
    return 1


async def dispatch_once(prompt: str, node_id: str):
    token = os.getenv("HEIWA_AUTH_TOKEN") or getattr(settings, "HEIWA_AUTH_TOKEN", "") or ""
    task_id = f"cli-task-{uuid.uuid4().hex[:8]}"
    fast_path = maybe_fast_path_turn(prompt, node_id)
    if fast_path:
        print(fast_path.response)
        sys.exit(0)

    for hub_url in hub_url_candidates():
        try:
            result = await _submit_to_hub(prompt, node_id, task_id, hub_url, token)
            if result:
                route = result.get("route", {})
                accepted_id = result.get("task_id", task_id)
                print(f"[HEIWA] Accepted by hub. Task: {accepted_id}")
                if hub_url != hub_url_candidates()[0]:
                    print(f"[HEIWA] Active hub fallback: {hub_url}")
                print(f"[HEIWA] Route: {route.get('intent_class')} -> {route.get('target_tool')} ({route.get('target_model', 'default')})")
                exit_code = await _stream_task_result(hub_url, accepted_id, token=token)
                sys.exit(exit_code)
        except Exception as e:
            print(f"[HEIWA] Hub unreachable ({hub_url}): {e}")

    # Fallback to direct local execution
    print("[HEIWA] Falling back to direct local route.")
    sys.exit(await _direct_execute(prompt, node_id, task_id, token))


if __name__ == "__main__":
    if len(sys.argv) < 3:
        print("Usage: dispatch_once.py <node_id> <prompt>")
        sys.exit(1)
    node_id = sys.argv[1]
    prompt = " ".join(sys.argv[2:])
    asyncio.run(dispatch_once(prompt, node_id))
