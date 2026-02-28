import asyncio
import json
import logging
import time
from pathlib import Path
from typing import Any, Dict, List
from fastapi import FastAPI, HTTPException
from fastapi.responses import FileResponse
from fastapi.staticfiles import StaticFiles
from pydantic import BaseModel
from heiwa_sdk.db import Database
from heiwa_sdk import settings, MCPBridge, redact_any, load_swarm_env

# Initialized Environment
load_swarm_env()

logger = logging.getLogger("Hub.MCP")
app = FastAPI(title="Heiwa Core MCP Server")
db = Database()
ROOT = Path(__file__).resolve().parents[2]
WEB_ROOT = ROOT / "apps" / "heiwa_web" / "clients" / "web"
ASSETS_ROOT = WEB_ROOT / "assets"

if ASSETS_ROOT.exists():
    app.mount("/assets", StaticFiles(directory=str(ASSETS_ROOT)), name="assets")

mcp_bridge = MCPBridge()


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
    return {"status": "alive", "service": "heiwa-core-hub", "timestamp": time.time()}


@app.get("/")
@app.head("/")
async def root():
    index = _web_file("index.html")
    if index:
        return FileResponse(index)
    return {"name": "Heiwa Limited", "version": "1.0.0", "status": "operational"}


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

@app.get("/canvas")
@app.get("/canvas/")
async def canvas_page():
    page = _web_file("canvas/index.html")
    if page:
        return FileResponse(page)
    raise HTTPException(status_code=404, detail="canvas artifact unavailable")

@app.get("/status")
async def get_public_status():
    """Public telemetry snapshot for the web dashboard."""
    try:
        summary = db.get_model_usage_summary(minutes=60)
    except Exception as exc:
        logger.warning("Public status summary unavailable: %s", exc)
        summary = []
    return {
        "status": "OPERATIONAL",
        "mesh_nodes": ["macbook@heiwa-agile", "wsl@heiwa-thinker", "railway@mesh-brain"],
        "active_models": len(summary),
        "timestamp": time.time()
    }

@app.get("/tools")
async def list_tools():
    """List available swarm management tools and bridged MCP tools."""
    # 1. Native Heiwa Tools
    native_tools = [
        {
            "name": "heiwa_get_swarm_status",
            "description": "Retrieve health and resource usage for all active nodes in the mesh.",
            "input_schema": {"type": "object", "properties": {}}
        },
        {
            "name": "heiwa_get_latest_tasks",
            "description": "Fetch the 10 most recent swarm directives and their statuses.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "limit": {"type": "integer", "default": 10}
                }
            }
        }
    ]
    
    # 2. Bridged MCP Tools
    try:
        bridged_tools = mcp_bridge.list_tools()
        # Add a prefix to bridged tools to avoid collisions? 
        # Or just return them as is if we want transparent bridging.
    except Exception as exc:
        logger.error("Failed to list bridged tools: %s", exc)
        bridged_tools = []
        
    return {"native": native_tools, "bridged": bridged_tools}

@app.post("/call/{tool_name}")
async def call_tool(tool_name: str, arguments: Dict[str, Any]):
    """Execute a swarm management tool (Native or Bridged)."""
    
    # Redact input for audit/safety
    safe_args = redact_any(arguments)
    
    if tool_name == "heiwa_get_swarm_status":
        summary = db.get_model_usage_summary(minutes=60)
        return {"content": [{"type": "text", "text": json.dumps(summary, indent=2)}]}
    
    elif tool_name == "heiwa_get_latest_tasks":
        limit = safe_args.get("limit", 10)
        conn = db.get_connection()
        cursor = conn.cursor()
        cursor.execute("SELECT * FROM runs ORDER BY ended_at DESC LIMIT %s", (limit,))
        rows = cursor.fetchall()
        tasks = db._rows_to_dicts(rows, cursor)
        conn.close()
        return {"content": [{"type": "text", "text": json.dumps(tasks, indent=2)}]}
    
    # 3. Fallback to Bridged MCP
    try:
        result = mcp_bridge.call_tool(tool_name, safe_args)
        if result["ok"]:
            return {"content": [{"type": "text", "text": json.dumps(result["result"], indent=2)}]}
        else:
            raise HTTPException(status_code=500, detail=result.get("stderr", "MCP tool failed"))
    except Exception as e:
        if "Tool not found" in str(e):
             raise HTTPException(status_code=404, detail="Tool not found")
        raise

def start_mcp_server():
    import uvicorn
    logger.info("📡 Heiwa MCP Server booting on port 8000...")
    uvicorn.run(app, host="0.0.0.0", port=8000)

if __name__ == "__main__":
    start_mcp_server()
