import asyncio
import json
import logging
import time
from pathlib import Path
from typing import Any, Dict

import os
import uuid

from fastapi import FastAPI, HTTPException, Header, WebSocket, WebSocketDisconnect
from fastapi.responses import FileResponse
from fastapi.staticfiles import StaticFiles
from pydantic import BaseModel

from heiwa_sdk import (
    HubStateService,
    MCPBridge,
    Database,
    HeiwaBench,
    HeiwaCellCatalog,
    redact_any,
    load_swarm_env,
)
from heiwa_protocol.routing import BrokerRouteRequest

from apps.heiwa_hub.cognition.enrichment import BrokerEnrichmentService
from apps.heiwa_hub.transport import get_bus, get_worker_manager
from heiwa_protocol.protocol import Subject

# Initialized Environment
load_swarm_env()

logger = logging.getLogger("Hub.MCP")
app = FastAPI(title="Heiwa Core MCP Server")
db = Database()
state = HubStateService(db)
mcp_bridge = MCPBridge()
enrichment = BrokerEnrichmentService()

ROOT = Path(__file__).resolve().parents[2]
bench = HeiwaBench(ROOT)
cells = HeiwaCellCatalog(ROOT)
WEB_ROOT = ROOT / "apps" / "heiwa_web" / "clients" / "web"
ASSETS_ROOT = WEB_ROOT / "assets"

if ASSETS_ROOT.exists():
    app.mount("/assets", StaticFiles(directory=str(ASSETS_ROOT)), name="assets")


def _web_file(name: str) -> Path | None:
    candidate = WEB_ROOT / name
    return candidate if candidate.exists() else None


class MCPTool(BaseModel):
    name: str
    description: str
    input_schema: Dict[str, Any]


@app.get("/health")
@app.head("/health")
async def health():
    return {
        "status": "alive",
        "service": "heiwa-core-hub",
        "state_backend": db.state_backend,
        "gateway_transport": "websocket",
        "timestamp": time.time(),
    }


@app.get("/")
@app.head("/")
async def root():
    index = _web_file("index.html")
    if index:
        return FileResponse(index)
    return {"name": "Heiwa", "status": "operational"}


@app.get("/domains")
@app.get("/domains.html")
async def domains_page():
    page = _web_file("domains.html")
    if page:
        return FileResponse(page)
    raise HTTPException(status_code=404, detail="domains page unavailable")


@app.get("/governance")
@app.get("/governance.html")
async def governance_page():
    page = _web_file("governance.html")
    if page:
        return FileResponse(page)
    raise HTTPException(status_code=404, detail="governance page unavailable")


@app.get("/status.html")
async def status_page():
    page = _web_file("status.html")
    if page:
        return FileResponse(page)
    raise HTTPException(status_code=404, detail="status page unavailable")


@app.get("/status")
async def get_public_status():
    return state.get_public_status(minutes=60)


@app.websocket("/ws/status")
@app.websocket("/status/ws")
@app.websocket("/events")
async def status_stream(ws: WebSocket):
    await ws.accept()
    try:
        while True:
            await ws.send_json(state.get_public_status(minutes=60))
            await asyncio.sleep(2)
    except WebSocketDisconnect:
        logger.debug("Status websocket disconnected.")


@app.get("/tools")
async def list_tools():
    native_tools = [
        {
            "name": "heiwa_get_swarm_status",
            "description": "Retrieve the public Heiwa state snapshot.",
            "input_schema": {"type": "object", "properties": {}},
        },
        {
            "name": "heiwa_get_latest_tasks",
            "description": "Fetch recent Heiwa task runs from the active state backend.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "default": 10},
                },
            },
        },
        {
            "name": "heiwa_resolve_route",
            "description": "Resolve a raw task into the typed HeiwaClaw route contract.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "task_id": {"type": "string"},
                    "request_id": {"type": "string"},
                    "raw_text": {"type": "string"},
                    "source_surface": {"type": "string", "default": "cli"},
                    "privacy_level": {"type": "string"},
                },
                "required": ["raw_text"],
            },
        },
        {
            "name": "heiwa_run_bench",
            "description": "Run the HeiwaBench release-gate suites for routes and cell selection.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "suite": {"type": "string", "description": "Optional suite name such as routing_matrix or cells_catalog"}
                },
            },
        },
        {
            "name": "heiwa_get_cells_catalog",
            "description": "List the current HeiwaCells catalog and optionally recommend a cell for a prompt.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "prompt": {"type": "string", "description": "Optional prompt to match against the cell catalog"}
                },
            },
        },
    ]

    try:
        bridged_tools = mcp_bridge.list_tools()
    except Exception as exc:
        logger.error("Failed to list bridged tools: %s", exc)
        bridged_tools = []

    return {"native": native_tools, "bridged": bridged_tools}


@app.post("/call/{tool_name}")
async def call_tool(tool_name: str, arguments: Dict[str, Any]):
    safe_args = redact_any(arguments)

    if tool_name == "heiwa_get_swarm_status":
        return {"content": [{"type": "text", "text": json.dumps(state.get_public_status(minutes=60), indent=2)}]}

    if tool_name == "heiwa_get_latest_tasks":
        limit = int(arguments.get("limit", 10) or 10)
        tasks = state.get_recent_runs(limit=limit)
        return {"content": [{"type": "text", "text": json.dumps(tasks, indent=2)}]}

    if tool_name == "heiwa_resolve_route":
        request = BrokerRouteRequest.from_payload(
            {
                "request_id": arguments.get("request_id", f"mcp-route-{int(time.time() * 1000)}"),
                "task_id": arguments.get("task_id", f"mcp-task-{int(time.time() * 1000)}"),
                "raw_text": arguments.get("raw_text", ""),
                "source_surface": arguments.get("source_surface", "cli"),
                "privacy_level": arguments.get("privacy_level"),
                "auth_validated": True,
                "timestamp": time.time(),
            }
        )
        result = enrichment.enrich(request)
        return {"content": [{"type": "text", "text": json.dumps(result.to_dict(), indent=2)}]}

    if tool_name == "heiwa_run_bench":
        suite = arguments.get("suite")
        result = bench.run(suite=str(suite) if suite else None)
        return {"content": [{"type": "text", "text": json.dumps(result, indent=2)}]}

    if tool_name == "heiwa_get_cells_catalog":
        prompt = str(arguments.get("prompt") or "").strip()
        payload: dict[str, Any] = cells.to_public_dict()
        if prompt:
            payload["recommendation"] = cells.recommend(prompt)
        return {"content": [{"type": "text", "text": json.dumps(payload, indent=2)}]}

    try:
        result = mcp_bridge.call_tool(tool_name, safe_args)
        if result["ok"]:
            return {"content": [{"type": "text", "text": json.dumps(result["result"], indent=2)}]}
        raise HTTPException(status_code=500, detail=result.get("stderr", "MCP tool failed"))
    except Exception as exc:
        if "Tool not found" in str(exc):
            raise HTTPException(status_code=404, detail="Tool not found")
        raise


# ---------------------------------------------------------------------------
#  Auth helper
# ---------------------------------------------------------------------------

def _validate_auth_token(token: str | None) -> str:
    """Validate the Authorization bearer token against HEIWA_AUTH_TOKEN.
    Returns the expected token on success, raises HTTPException on failure."""
    expected = os.getenv("HEIWA_AUTH_TOKEN", "")
    if not expected:
        raise HTTPException(status_code=500, detail="Hub auth not configured (HEIWA_AUTH_TOKEN unset)")
    if not token:
        raise HTTPException(status_code=401, detail="Missing Authorization header")
    # Accept "Bearer <token>" or raw token
    raw = token.removeprefix("Bearer ").strip() if token.startswith("Bearer ") else token.strip()
    if raw != expected:
        raise HTTPException(status_code=403, detail="Invalid auth token")
    return expected


# ---------------------------------------------------------------------------
#  Task ingress
# ---------------------------------------------------------------------------

class TaskRequest(BaseModel):
    raw_text: str
    sender_id: str = "cli"
    source_surface: str = "cli"
    privacy_level: str | None = None
    task_id: str | None = None


@app.post("/tasks")
async def create_task(req: TaskRequest, authorization: str | None = Header(None)):
    """Authenticated task ingress. Enriches once here — Spine skips re-enrichment."""
    auth_token = _validate_auth_token(authorization)
    task_id = req.task_id or f"cli-task-{uuid.uuid4().hex[:8]}"

    # Single source of truth: enrich here, pass route fields through the bus
    broker_req = BrokerRouteRequest.from_payload({
        "request_id": f"http-{task_id}",
        "task_id": task_id,
        "raw_text": req.raw_text,
        "sender_id": req.sender_id,
        "source_surface": req.source_surface,
        "privacy_level": req.privacy_level,
        "auth_validated": True,
        "timestamp": time.time(),
    })
    route = enrichment.enrich(broker_req)

    # Publish to local bus — include auth_token so Spine's barrier passes,
    # and include enrichment fields so Spine skips double-enrichment.
    bus = get_bus()
    await bus.publish(Subject.CORE_REQUEST, {
        "task_id": task_id,
        "raw_text": req.raw_text,
        "source": req.source_surface,
        "sender_id": req.sender_id,
        "auth_token": auth_token,
        "intent_class": route.intent_class,
        "target_runtime": route.target_runtime,
        "target_tool": route.target_tool,
        "target_model": route.target_model,
        "compute_class": route.compute_class,
        "assigned_worker": route.assigned_worker,
        "risk_level": route.risk_level,
        "privacy_level": route.privacy_level,
        "normalization": route.normalization if hasattr(route, "normalization") else {},
        "_pre_enriched": True,
    }, sender_id=req.sender_id)

    return {
        "task_id": task_id,
        "status": "ACCEPTED",
        "route": route.to_dict(),
    }


@app.get("/tasks/{task_id}")
async def get_task(task_id: str, authorization: str | None = Header(None)):
    """Retrieve task status/results from state backend."""
    _validate_auth_token(authorization)
    runs = state.get_recent_runs(limit=50)
    for run in runs:
        if run.get("proposal_id") == task_id or run.get("run_id", "").endswith(task_id):
            return run
    raise HTTPException(status_code=404, detail=f"Task {task_id} not found")


@app.websocket("/ws/tasks/{task_id}")
async def task_events(ws: WebSocket, task_id: str, token: str | None = None):
    """Stream task events for a specific task to the CLI.

    Auth is via query parameter: ``ws://.../ws/tasks/{id}?token=<bearer>``.
    """
    # --- Auth gate ---
    expected = os.getenv("HEIWA_AUTH_TOKEN", "")
    if not expected or not token or token != expected:
        await ws.close(code=4003)
        return

    await ws.accept()
    event_queue: asyncio.Queue = asyncio.Queue()

    async def _capture(data: Dict[str, Any]):
        payload = data.get("data", data)
        if payload.get("task_id") == task_id:
            await event_queue.put(payload)

    bus = get_bus()
    await bus.subscribe(Subject.TASK_STATUS, _capture)
    await bus.subscribe(Subject.TASK_EXEC_RESULT, _capture)
    await bus.subscribe(Subject.TASK_PROGRESS, _capture)

    terminal_statuses = {"DELIVERED", "PASS", "FAIL", "BLOCKED_AUTH", "BLOCKED_NO_CONTENT"}
    try:
        while True:
            try:
                event = await asyncio.wait_for(event_queue.get(), timeout=30.0)
                await ws.send_json(event)
                if event.get("status") in terminal_statuses:
                    break
            except asyncio.TimeoutError:
                await ws.send_json({"type": "heartbeat", "task_id": task_id})
    except WebSocketDisconnect:
        pass
    finally:
        # Unsubscribe to avoid leaked callbacks
        for subj in (Subject.TASK_STATUS, Subject.TASK_EXEC_RESULT, Subject.TASK_PROGRESS):
            bus.unsubscribe(subj, _capture)


# ---------------------------------------------------------------------------
#  Worker WebSocket — hybrid push/pull for Mac/WSL nodes
# ---------------------------------------------------------------------------

@app.websocket("/ws/worker")
async def worker_socket(ws: WebSocket):
    """
    Remote worker registration and task delivery.

    Protocol:
      1. Worker sends: {"type": "register", "worker_id": "...", "auth_token": "...", "capabilities": {...}}
      2. Hub pushes:   {"type": "task_assignment", "data": {...}}
      3. Worker sends: {"type": "result", "task_id": "...", "status": "...", "summary": "..."}
      4. Worker sends: {"type": "heartbeat"} periodically
    """
    await ws.accept()
    wm = get_worker_manager()
    worker_id = None
    authenticated = False
    expected_token = os.getenv("HEIWA_AUTH_TOKEN", "")

    try:
        while True:
            raw = await ws.receive_json()
            msg_type = raw.get("type", "")

            if msg_type == "register":
                # Auth gate: first message must include valid token
                token = raw.get("auth_token", "")
                if not expected_token or token != expected_token:
                    await ws.send_json({"type": "error", "detail": "Invalid auth token"})
                    await ws.close(code=4003)
                    return
                authenticated = True
                worker_id = raw.get("worker_id", f"worker-{uuid.uuid4().hex[:8]}")
                wm.register(worker_id, ws, raw.get("capabilities", {}))
                await ws.send_json({"type": "registered", "worker_id": worker_id})

            elif not authenticated:
                await ws.send_json({"type": "error", "detail": "Must register with auth_token first"})
                await ws.close(code=4001)
                return

            elif msg_type == "heartbeat":
                if worker_id:
                    wm.heartbeat(worker_id)

            elif msg_type == "result":
                bus = get_bus()
                await bus.publish(Subject.TASK_EXEC_RESULT, raw.get("data", raw), sender_id=worker_id or "worker")

            elif msg_type == "pull":
                await ws.send_json({"type": "no_work"})

    except WebSocketDisconnect:
        if worker_id:
            wm.unregister(worker_id)
    except Exception as exc:
        logger.error("Worker socket error: %s", exc)
        if worker_id:
            wm.unregister(worker_id)


def start_mcp_server():
    import uvicorn
    logger.info("Heiwa MCP Server booting on port 8000...")
    uvicorn.run(app, host="0.0.0.0", port=8000)


if __name__ == "__main__":
    start_mcp_server()
