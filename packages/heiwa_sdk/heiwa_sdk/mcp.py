import json
import logging
import subprocess
import time
from typing import List, Dict, Any, Optional, Tuple
from .utils import run_cmd
from .security import redact_text

logger = logging.getLogger("SDK.MCP")

class MCPBridge:
    """
    Bridge to Model Context Protocol (MCP) servers.
    Primarily uses the 'docker mcp' CLI for discovery and execution.
    """
    def __init__(self, root_dir: Optional[str] = None):
        self.root_dir = root_dir

    def list_tools(self, timeout: int = 25) -> List[Dict[str, Any]]:
        """List all available MCP tools via docker mcp."""
        result = run_cmd(["docker", "mcp", "tools", "ls", "--format", "json"], timeout=timeout)
        if result.returncode != 0:
            logger.error(f"Failed to list MCP tools: {result.stderr}")
            return []
        
        try:
            payload = json.loads(result.stdout)
            if isinstance(payload, list):
                return payload
            if isinstance(payload, dict) and "tools" in payload:
                return payload["tools"]
        except json.JSONDecodeError:
            logger.error("Failed to parse MCP tools catalog")
            
        return []

    def call_tool(self, tool_name: str, arguments: Dict[str, Any], timeout: int = 60) -> Dict[str, Any]:
        """Call an MCP tool."""
        args = ["docker", "mcp", "tools", "call", tool_name]
        for key, value in arguments.items():
            if value is None: continue
            # Convert values to CLI-friendly format
            val_str = str(value)
            if isinstance(value, (dict, list)):
                val_str = json.dumps(value, separators=(",", ":"))
            elif isinstance(value, bool):
                val_str = "true" if value else "false"
            
            args.append(f"{key}={val_str}")

        logger.info(f"🔌 [MCP] Calling tool: {tool_name}")
        result = run_cmd(args, timeout=timeout)
        
        # Parse stdout for 'Tool call took: ...ms'
        stdout_clean = result.stdout.strip()
        cli_duration_ms = None
        lines = stdout_clean.splitlines()
        for line in lines:
            if "Tool call took:" in line:
                try:
                    cli_duration_ms = int(float(line.split(":")[1].replace("ms", "").strip()))
                except: pass
                stdout_clean = stdout_clean.replace(line, "").strip()

        try:
            parsed_result = json.loads(stdout_clean)
        except:
            parsed_result = stdout_clean

        return {
            "ok": result.returncode == 0,
            "tool": tool_name,
            "duration_ms": cli_duration_ms or result.duration_ms,
            "result": parsed_result,
            "stderr": redact_text(result.stderr) if result.returncode != 0 else ""
        }
