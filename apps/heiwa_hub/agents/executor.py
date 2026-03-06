"""
Heiwa Executor Agent

Subscribes to NATS execution subjects and processes tasks using the
tiered LLM engine. This is the "hands" of the system — it receives
planned work and produces results.

Runs on Railway for API-routed tasks. Nodes run their own executor
subscribing to the same subjects filtered by target_runtime.
"""
import asyncio
import logging
import os
import sys
import time
from pathlib import Path
from typing import Any

from heiwa_hub.agents.base import BaseAgent
from heiwa_sdk.cognition import CognitionEngine
from heiwa_protocol.protocol import Subject
from heiwa_sdk.tool_mesh import ToolMesh

logger = logging.getLogger("Executor")

from heiwa_sdk.db import Database

class ExecutorAgent(BaseAgent):
    """
    Processes execution requests dispatched by the Planner/Messenger.
    """

    def __init__(self):
        super().__init__(name="heiwa-executor")
        self.executor_runtime = self._resolve_runtime()
        self.root = Path(__file__).resolve().parents[3]
        self.db = Database()
        self.engine = CognitionEngine(db=self.db)
        self.mesh = ToolMesh(self.root)

    async def run(self):
        """Boot the executor and start listening for work."""
        await self.connect()
        
        # Unified Execution Subject for SOTA v3.1 - Using Enum member
        await self.listen(Subject.TASK_EXEC, self._handle_exec)
        
        # Keep legacy listeners for backward compatibility - Using Enum members
        await self.listen(Subject.TASK_EXEC_REQUEST_CODE, self._handle_exec)
        await self.listen(Subject.TASK_EXEC_REQUEST_RESEARCH, self._handle_exec)
        
        logger.info(f"🦾 Executor Active ({self.executor_runtime}). Awaiting directives...")
        
        while self.running:
            await asyncio.sleep(1)

    async def _handle_exec(self, data: dict[str, Any]) -> None:
        """Process a single execution request with extreme transparency."""
        payload = data.get("data", data)
        task_id = payload.get("task_id", "unknown")
        instruction = payload.get("instruction", "")
        intent_class = payload.get("intent_class", "general")
        target_runtime = payload.get("target_runtime", "railway").lower()
        target_model = payload.get("target_model", "google/gemini-2.0-flash")
        target_tool = str(payload.get("target_tool", "ollama")).strip().lower() or "ollama"

        if target_runtime not in {self.executor_runtime, "both", "any"}:
            logger.info(f"⏭️  [EXECUTOR] Skipping task {task_id}: target={target_runtime}, local={self.executor_runtime}")
            return

        logger.info(f"🚀 [EXECUTOR] Processing Task: {task_id} | Intent: {intent_class}")
        await self.speak(Subject.LOG_INFO, {"message": f"🦾 Executor {self.name} starting task {task_id}", "task_id": task_id})
        await self.speak(
            Subject.TASK_STATUS,
            {
                "accepted": True,
                "reason": None,
                "task_id": task_id,
                "step_id": payload.get("step_id", "executor"),
                "status": "RUNNING",
                "message": f"Executor {self.name} started task execution.",
                "runtime": self.executor_runtime,
                "response_channel_id": payload.get("response_channel_id"),
                "response_thread_id": payload.get("response_thread_id"),
            },
        )

        start = time.time()
        full_result = ""
        exec_status = "FAIL"
        
        try:
            if target_tool == "heiwa_ops" and intent_class == "audit":
                logger.info("🧪 [EXECUTOR] Routing %s through bounded Class 1 audit check.", task_id)
                exec_status, full_result = await self._run_bounded_audit(instruction)
            elif target_tool == "heiwa_ops":
                logger.info("🛠️  [EXECUTOR] Routing %s through bounded local ops tool.", task_id)
                code, output = await self.mesh.execute(target_tool, instruction, model=target_model)
                exec_status = "PASS" if code == 0 else "FAIL"
                full_result = output.strip()
            else:
                logger.info(f"🧠 [EXECUTOR] Initiating SOTA Cognition for {task_id}...")
                # Use evaluate_and_stream for reasoning + conscience + execution
                async for chunk in self.engine.evaluate_and_stream(
                    origin=self.name,
                    intent=intent_class,
                    instruction=instruction,
                    model=target_model,
                    system=self._build_intent_context(intent_class)
                ):
                    full_result += chunk
                    # Broadcast every thought chunk back to the CLI
                    await self.speak(Subject.LOG_THOUGHT, {
                        "task_id": task_id,
                        "agent": self.name,
                        "content": chunk,
                        "stream": True
                    })
                exec_status = "PASS" if full_result else "FAIL"
                logger.info(f"✅ [EXECUTOR] Generation complete for {task_id}.")
        except Exception as e:
            logger.error(f"❌ [EXECUTOR] Task {task_id} FAILED: {e}")
            full_result = f"Error: {e}"
            await self.speak(Subject.LOG_ERROR, {"content": str(e), "task_id": task_id})
            exec_status = "FAIL"

        elapsed = round(time.time() - start, 2)
        logger.info(f"🏁 [EXECUTOR] Task {task_id} finished in {elapsed}s.")

        # Publish final result
        result_payload = {
            "task_id": task_id,
            "status": exec_status,
            "summary": full_result,
            "runtime": self.executor_runtime,
            "elapsed_sec": elapsed,
            "target_tool": target_tool,
            "intent_class": intent_class,
            "requested_by": payload.get("requested_by"),
            "response_channel_id": payload.get("response_channel_id"),
            "response_thread_id": payload.get("response_thread_id"),
        }

        await self.speak(Subject.TASK_EXEC_RESULT, result_payload)
        await self.speak(
            Subject.TASK_STATUS,
            {
                "accepted": exec_status == "PASS",
                "reason": None if exec_status == "PASS" else "Executor produced a failure result.",
                "task_id": task_id,
                "step_id": payload.get("step_id", "executor"),
                "status": "DELIVERED",
                "message": f"Execution result published to {Subject.TASK_EXEC_RESULT.value} ({exec_status}).",
                "runtime": self.executor_runtime,
                "target_tool": target_tool,
                "response_channel_id": payload.get("response_channel_id"),
                "response_thread_id": payload.get("response_thread_id"),
            },
        )

    def _resolve_runtime(self) -> str:
        explicit = str(os.getenv("HEIWA_EXECUTOR_RUNTIME", "")).strip().lower()
        if explicit:
            return explicit
        if os.getenv("RAILWAY_ENVIRONMENT") or os.getenv("RAILWAY_PROJECT_ID"):
            return "railway"
        return "macbook"

    async def _run_bounded_audit(self, instruction: str) -> tuple[str, str]:
        script = self.root / "apps/heiwa_cli/scripts/lint_config.py"
        if not script.exists():
            return "FAIL", f"Missing audit script: {script}"

        proc = await asyncio.create_subprocess_exec(
            sys.executable,
            str(script),
            cwd=str(self.root),
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        stdout, stderr = await proc.communicate()
        combined = (stdout + stderr).decode(errors="ignore").strip()
        if not combined and proc.returncode == 0:
            combined = "PASS: lint_config"
        status = "PASS" if proc.returncode == 0 else "FAIL"
        probe_marker = self._extract_smoke_probe(instruction)
        summary = f"[CLASS1_AUDIT] lint_config.py\n[EXIT_CODE] {proc.returncode}\n{combined}".strip()
        if probe_marker:
            summary = f"{summary}\nHEIWA_SMOKE_PROBE_OK:{probe_marker}"
        return status, summary

    @staticmethod
    def _extract_smoke_probe(instruction: str) -> str:
        marker = "HEIWA_SMOKE_PROBE:"
        if marker not in instruction:
            return ""
        suffix = instruction.split(marker, 1)[1].strip()
        return suffix.split()[0] if suffix else ""

    @staticmethod
    def _build_intent_context(intent_class: str) -> str:
        """Return a focused context augmentation for the given intent."""
        contexts = {
            "build": "Focus: SOTA Software Engineering. Implementation must be clean, production-ready, and efficient.",
            "research": "Focus: Meticulous Analysis. Findings must be sourced, structured, and highlight confidence levels.",
            "deploy": "Focus: DevOps & Infrastructure. Safety first. Include verification and rollback considerations.",
            "operate": "Focus: Systems Operation. Calculated diagnostics. Provide exact, verified commands.",
            "automation": "Focus: Idempotent Workflows. Reliable, maintainable, and simple solutions.",
        }
        return contexts.get(intent_class, "Focus: Efficient and precise execution.")
