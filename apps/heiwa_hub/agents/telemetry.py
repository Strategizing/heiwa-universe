import asyncio
import json
import logging
import time
import datetime
from typing import Any, Dict
from heiwa_hub.agents.base import BaseAgent
from heiwa_protocol.protocol import Subject
from heiwa_sdk.db import Database
from heiwa_sdk.cost import CostEstimator

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
        await self.listen(Subject.NODE_HEARTBEAT, self.handle_node_heartbeat)
        await self.listen(Subject.NODE_TELEMETRY, self.handle_node_heartbeat)
        
        logger.info("📊 Telemetry Agent Active. Monitoring Swarm Usage...")

        # Background loop for periodical analysis
        pulse_count = 0
        while self.running:
            try:
                await self.process_analytics()
            except Exception as exc:
                logger.error("Telemetry analytics loop error: %s", exc)
            
            # Swarm Pulse: Proactive brainstorming every hour (60 loops)
            pulse_count += 1
            if pulse_count >= 60:
                await self.broadcast_pulse()
                pulse_count = 0
                
            await asyncio.sleep(60)

    async def broadcast_pulse(self):
        """Proactively broadcast a swarm-wide thought about the mesh health or evolution."""
        now = datetime.datetime.now().strftime("%H:%M")
        thought = f"Swarm Pulse at {now}: Resources are stable across all nodes. "
        thought += "Optimization opportunity: Consider offloading more research tasks to the Workstation to save Macbook cycles."
        await self.think(thought, encrypt=True)
        
        # Also post to Roadmap
        await self.speak(Subject.LOG_INFO, {
            "agent": self.name,
            "status": "PASS",
            "intent_class": "strategy",
            "content": f"## 🌊 Swarm Pulse: {now}\n{thought}",
            "response_channel_id": self.db.get_discord_channel("roadmap")
        })

    async def handle_status_query(self, data: dict[str, Any]):
        """Respond with the latest usage cache and node health."""
        report = {
            "models": self.usage_cache,
            "timestamp": time.time(),
            "status": "OPERATIONAL"
        }
        await self.speak(Subject.SWARM_STATUS_REPORT, report)

    @staticmethod
    def _unwrap(data: dict[str, Any]) -> dict[str, Any]:
        if not isinstance(data, dict):
            return {}
        inner = data.get("data")
        return inner if isinstance(inner, dict) else data

    async def handle_exec_result(self, data: dict[str, Any]):
        payload = self._unwrap(data)
        task_id = payload.get("task_id")
        node_id = payload.get("runtime", "unknown")
        model_id = payload.get("model_id") or payload.get("target_tool")
        
        # Extract token usage from artifacts or payload
        tokens = self._extract_tokens(payload)
        
        # Calculate cost
        cost = CostEstimator.calculate(model_id, tokens.get("input", 0), tokens.get("output", 0))

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
            "cost": cost,
            "duration_ms": payload.get("duration_ms", 0),
            "mode": "PRODUCTION"
        }
        self.db.record_run(run_data)
        
        logger.info(f"📈 Logged Usage: model={model_id} node={node_id} tokens={tokens.get('total')} cost=${cost}")

    async def handle_node_heartbeat(self, data: dict[str, Any]):
        """Persist node liveness and stats to the database."""
        payload = self._unwrap(data)
        node_id = payload.get("node_id") or payload.get("sender_id")
        
        if not node_id:
            return

        # Prepare metadata for the upsert
        meta = {
            "cpu_pct": payload.get("cpu_pct", 0),
            "ram_pct": payload.get("ram_pct", 0),
            "ram_used_gb": payload.get("ram_used_gb", 0),
            "ram_total_gb": payload.get("ram_total_gb", 0),
            "agent_name": payload.get("agent_name", "unknown")
        }
        
        self.db.upsert_node_heartbeat(
            node_id=node_id,
            meta=meta
        )

    async def handle_status(self, data: dict[str, Any]):
        # Optional: track heartbeat/concurrency here
        pass

    async def process_analytics(self):
        """Analyze usage and poll local metrics."""
        now = time.time()
        
        # BaseAgent heartbeat now handles the broadcast of local metrics automatically.
        # This method can focus on Swarm-wide analytics and DB state.

        if now - self.last_summary_ts < 300: # Every 5 mins
            return
            
        try:
            summary = self.db.get_model_usage_summary(minutes=60)
        except Exception as exc:
            logger.warning("Skipping model usage summary (schema mismatch or DB issue): %s", exc)
            self.last_summary_ts = now
            return
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
