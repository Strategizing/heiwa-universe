import asyncio
import json
import logging
import time
from typing import Any, Dict
from fleets.hub.agents.base import BaseAgent
from fleets.hub.protocol import Subject
from libs.heiwa_sdk.db import Database

logger = logging.getLogger("Telemetry")

class TelemetryAgent(BaseAgent):
    """
    Railway vCPU task: Always-on monitor for the Heiwa Swarm.
    Tracks token usage, model performance, and live rate limits.
    """
    def __init__(self):
        super().__init__(name="heiwa-telemetry")
        self.db = Database()
        # In-memory cache for live rate limits
        self.usage_cache: Dict[str, Dict[str, Any]] = {}
        self.last_summary_ts = 0

    async def run(self):
        await self.connect()
        
        # Listen for all execution results across the mesh
        await self.listen(Subject.TASK_EXEC_RESULT, self.handle_exec_result)
        await self.listen(Subject.TASK_STATUS, self.handle_status)
        await self.listen(Subject.SWARM_STATUS_QUERY, self.handle_status_query)
        
        logger.info("📊 Telemetry Agent Active. Monitoring Swarm Usage...")

        # Background loop for periodical analysis
        while self.running:
            await self.process_analytics()
            await asyncio.sleep(60)

    async def handle_status_query(self, data: dict[str, Any]):
        """Respond with the latest usage cache and node health."""
        report = {
            "models": self.usage_cache,
            "timestamp": time.time(),
            "status": "OPERATIONAL"
        }
        await self.speak(Subject.SWARM_STATUS_REPORT, report)

    async def handle_exec_result(self, data: dict[str, Any]):
        payload = self._unwrap(data)
        task_id = payload.get("task_id")
        node_id = payload.get("runtime", "unknown")
        model_id = payload.get("model_id") or payload.get("target_tool")
        
        # Extract token usage from artifacts or payload
        tokens = self._extract_tokens(payload)
        
        # Log to Sovereignty (DB)
        run_data = {
            "run_id": f"run-{int(time.time()*1000)}",
            "proposal_id": task_id,
            "started_at": None, # Could be improved if tracked
            "status": payload.get("status", "UNKNOWN"),
            "node_id": node_id,
            "model_id": model_id,
            "tokens_input": tokens.get("input", 0),
            "tokens_output": tokens.get("output", 0),
            "tokens_total": tokens.get("total", 0),
            "duration_ms": payload.get("duration_ms", 0),
            "mode": "PRODUCTION"
        }
        self.db.record_run(run_data)
        
        logger.info(f"📈 Logged Usage: model={model_id} node={node_id} tokens={tokens.get('total')}")

    async def handle_status(self, data: dict[str, Any]):
        # Optional: track heartbeat/concurrency here
        pass

    async def process_analytics(self):
        """Analyze usage and poll local Railway metrics."""
        now = time.time()
        
        # Poll Railway metrics (vCPU/RAM)
        import psutil
        railway_stats = {
            "node_id": "railway@mesh-brain",
            "cpu_pct": psutil.cpu_percent(),
            "ram_pct": psutil.virtual_memory().percent,
            "ram_used_gb": round(psutil.virtual_memory().used / (1024**3), 2),
            "ram_total_gb": round(psutil.virtual_memory().total / (1024**3), 2),
            "timestamp": now
        }
        await self.speak(Subject.NODE_TELEMETRY, railway_stats)

        if now - self.last_summary_ts < 300: # Every 5 mins
            return
            
        summary = self.db.get_model_usage_summary(minutes=60)
        # Update internal state
        for s in summary:
            mid = s["model_id"]
            self.usage_cache[mid] = {
                "requests_last_hour": s["request_count"],
                "tokens_last_hour": s["total_tokens"],
                "updated_at": now
            }
        
        self.last_summary_ts = now
        logger.info(f"📊 Processed Swarm-wide Analytics.")

    def _extract_tokens(self, payload: dict[str, Any]) -> dict[str, int]:
        """Attempt to find token usage in various payload formats."""
        # Check standard result/metadata
        usage = payload.get("usage", {})
        if usage:
            return {
                "input": usage.get("prompt_tokens", usage.get("input_tokens", 0)),
                "output": usage.get("completion_tokens", usage.get("output_tokens", 0)),
                "total": usage.get("total_tokens", 0)
            }
            
        # Check artifacts
        for art in payload.get("artifacts", []):
            if art.get("kind") == "usage":
                try:
                    val = json.loads(art["value"])
                    return {
                        "input": val.get("input", 0),
                        "output": val.get("output", 0),
                        "total": val.get("total", 0)
                    }
                except:
                    pass
        return {"input": 0, "output": 0, "total": 0}

if __name__ == "__main__":
    agent = TelemetryAgent()
    asyncio.run(agent.run())
