import asyncio
import json
import logging
import os
import sys
import time
import uuid
from pathlib import Path
from typing import Any, Dict, List, Optional

# Ensure enterprise monorepo roots are on sys.path
ROOT = Path(__file__).resolve().parents[4]
if str(ROOT / "packages/heiwa_sdk") not in sys.path:
    sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))
if str(ROOT / "packages/heiwa_protocol") not in sys.path:
    sys.path.insert(0, str(ROOT / "packages/heiwa_protocol"))
if str(ROOT / "apps") not in sys.path:
    sys.path.insert(0, str(ROOT / "apps"))

from heiwa_sdk.config import load_swarm_env
load_swarm_env()

from heiwa_hub.agents.base import BaseAgent
from heiwa_protocol.protocol import Subject
from heiwa_sdk.tool_mesh import ToolMesh

logger = logging.getLogger("WorkerManager")

class RateLimiter:
    """Tracks local usage per model to enforce soft rate limits before dispatching."""
    def __init__(self):
        self.usage: dict[str, list[float]] = {} 
        self.lock = asyncio.Lock()

    async def check(self, model_id: str, rpm_limit: int = 30) -> bool:
        async with self.lock:
            now = time.time()
            window = now - 60
            self.usage[model_id] = [ts for ts in self.usage.get(model_id, []) if ts > window]
            if len(self.usage[model_id]) >= rpm_limit:
                return False
            self.usage[model_id].append(now)
            return True

class ModelRouter:
    """Smart model selection based on identity fallback list and rate limits."""
    def __init__(self, limiter: RateLimiter):
        self.limiter = limiter

    async def select_best(self, models: list[str]) -> str | None:
        for model in models:
            limit = 1000 if "ollama" in model else 30
            if await self.limiter.check(model, rpm_limit=limit):
                return model
        return None

class WorkerManager(BaseAgent):
    """Enterprise-grade execution daemon supporting parallel tools and multi-node mesh."""

    def __init__(self) -> None:
        super().__init__(name="heiwa-worker-manager")
        self.root = ROOT
        self.node_id = os.getenv("HEIWA_NODE_ID", "macbook@heiwa-agile")
        self.node_type = os.getenv("HEIWA_NODE_TYPE", "mobile_node")
        self.limiter = RateLimiter()
        self.router = ModelRouter(self.limiter)
        self.mesh = ToolMesh(self.root)
        self.capabilities = {
            item.strip().lower()
            for item in os.getenv("HEIWA_CAPABILITIES", "").split(",")
            if item.strip()
        }
        if not self.capabilities:
            if self.node_type == "heavy_compute":
                self.capabilities = {"heavy_compute", "gpu_native", "standard_compute"}
            else:
                self.capabilities = {"standard_compute", "workspace_interaction", "agile_coding"}

        self.concurrency = int(os.getenv("HEIWA_EXECUTOR_CONCURRENCY", "4"))
        self.sem = asyncio.Semaphore(max(1, self.concurrency))

    async def execute(self, payload: dict[str, Any]) -> None:
        async with self.sem:
            start = time.time()
            task_id = str(payload.get("task_id", "unknown"))
            tool = str(payload.get("target_tool", "openclaw")).lower()
            instruction = str(payload.get("instruction") or payload.get("raw_text") or "").strip()

            # Capability & Node Check
            required_caps = set(payload.get("required_capabilities") or [])
            target_node = payload.get("target_runtime", "any")
            if target_node not in {self.node_id, "any", "all"} and not required_caps.issubset(self.capabilities):
                return

            # Identity & Model Routing
            try:
                from apps.heiwa_cli.scripts.ops.identity_selector import select_identity, load_profiles
                profiles = load_profiles()
                selection = select_identity(instruction, profiles)
                selected = selection.get("selected", {})
                
                # Check for "Cell" (Multi-Agent) Execution
                cells = selected.get("cells", [])
                if cells:
                    await self.execute_cell(task_id, cells, instruction, payload)
                    return # Cell execution handles its own result emitting

                models = selected.get("models", {}).get("openclaw", [])
                selected_model = await self.router.select_best(models)
            except Exception as e:
                logger.warning(f"Identity resolution failed: {e}")
                selected_model = None

            await self.think(f"Executing {tool} instance for task {task_id}", task_id=task_id)

            code, out = await self.mesh.execute(tool, instruction, model=selected_model)
            status = "PASS" if code == 0 else "FAIL"
            
            usage = {}
            try:
                data = json.loads(out)
                usage = data.get("usage", {})
                out = data.get("message", out)
            except: pass

            await self._emit_result(task_id, status, out, int((time.time()-start)*1000), usage, payload)

    async def execute_cell(self, task_id: str, cells: List[Dict[str, Any]], instruction: str, payload: dict):
        """Execute multiple agents/models in parallel for the same task."""
        logger.info(f"🧬 Spawning Cell Execution for task {task_id} with {len(cells)} cells.")
        
        async def run_cell(cell_def):
            role = cell_def.get("role")
            model = cell_def.get("model")
            cell_instruction = f"[CELL ROLE: {role}]\n{instruction}"
            
            await self.think(f"Cell Agent '{role}' starting reasoning...", task_id=task_id)
            code, out = await self.mesh.execute("openclaw", cell_instruction, model=model)
            return {"role": role, "model": model, "code": code, "output": out}

        tasks = [run_cell(c) for c in cells]
        results = await asyncio.gather(*tasks)
        
        # Consolidate results
        combined_summary = "## Swarm Cell Execution Summary\n\n"
        for r in results:
            status_icon = "✅" if r["code"] == 0 else "❌"
            combined_summary += f"### {status_icon} Agent: {r['role']} ({r['model']})\n{r['output'][:500]}...\n\n"
        
        await self._emit_result(task_id, "PASS", combined_summary, 0, {}, payload)

    async def _emit_result(self, task_id, status, summary, duration, usage, payload):
        result = {
            "task_id": task_id,
            "status": status,
            "summary": summary,
            "duration_ms": duration,
            "usage": usage,
            "runtime": self.node_id,
            "target_tool": payload.get("target_tool")
        }
        await self.speak(Subject.TASK_EXEC_RESULT, result)

    async def run(self):
        await self.connect()
        for sub in [Subject.TASK_EXEC_REQUEST_CODE, Subject.TASK_EXEC_REQUEST_RESEARCH, Subject.TASK_EXEC_REQUEST_OPERATE]:
            await self.listen(sub, self.execute)
        while True: await asyncio.sleep(1)

if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO)
    asyncio.run(WorkerManager().run())
