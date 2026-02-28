import asyncio
import json
import logging
from typing import Any, Dict, List
from fastapi import FastAPI, HTTPException
from pydantic import BaseModel
from heiwa_sdk.db import Database
from heiwa_sdk.config import load_swarm_env

# Initialized Environment
load_swarm_env()

logger = logging.getLogger("Hub.MCP")
app = FastAPI(title="Heiwa Core MCP Server")
db = Database()

class MCPTool(BaseModel):
    name: str
    description: str
    input_schema: Dict[str, Any]

@app.get("/status")
async def get_public_status():
    """Public telemetry snapshot for the web dashboard."""
    summary = db.get_model_usage_summary(minutes=60)
    return {
        "status": "OPERATIONAL",
        "mesh_nodes": ["macbook@heiwa-agile", "wsl@heiwa-thinker", "railway@mesh-brain"],
        "active_models": len(summary),
        "timestamp": time.time()
    }

@app.get("/tools")
async def list_tools():
    """List available swarm management tools for agents."""
    return {
        "tools": [
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
    }

@app.post("/call/{tool_name}")
async def call_tool(tool_name: str, arguments: Dict[str, Any]):
    """Execute a swarm management tool."""
    if tool_name == "heiwa_get_swarm_status":
        # Pull latest telemetry from DB
        summary = db.get_model_usage_summary(minutes=60)
        return {"content": [{"type": "text", "text": json.dumps(summary, indent=2)}]}
    
    elif tool_name == "heiwa_get_latest_tasks":
        limit = arguments.get("limit", 10)
        conn = db.get_connection()
        cursor = conn.cursor()
        cursor.execute("SELECT * FROM runs ORDER BY ended_at DESC LIMIT %s", (limit,))
        rows = cursor.fetchall()
        tasks = db._rows_to_dicts(rows, cursor)
        conn.close()
        return {"content": [{"type": "text", "text": json.dumps(tasks, indent=2)}]}
    
    else:
        raise HTTPException(status_code=404, detail="Tool not found")

def start_mcp_server():
    import uvicorn
    logger.info("📡 Heiwa MCP Server booting on port 8000...")
    uvicorn.run(app, host="0.0.0.0", port=8000)

if __name__ == "__main__":
    start_mcp_server()
