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
import time
from typing import Any

from heiwa_hub.agents.base import BaseAgent
from heiwa_sdk.cognition import CognitionEngine
from heiwa_protocol.protocol import Subject

logger = logging.getLogger("Executor")

from heiwa_sdk.db import Database

class ExecutorAgent(BaseAgent):
    """
    Processes execution requests dispatched by the Planner/Messenger.
    """

    def __init__(self):
        super().__init__(name="heiwa-executor")
        self.executor_runtime = str(os.getenv("HEIWA_EXECUTOR_RUNTIME", "railway")).strip().lower() or "railway"
        self.db = Database()
        self.engine = CognitionEngine(db=self.db)

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

        if target_runtime not in {self.executor_runtime, "both", "any"}:
            logger.info(f"⏭️  [EXECUTOR] Skipping task {task_id}: target={target_runtime}, local={self.executor_runtime}")
            return

        logger.info(f"🚀 [EXECUTOR] Processing Task: {task_id} | Intent: {intent_class}")
        await self.speak(Subject.LOG_INFO, {"message": f"🦾 Executor {self.name} starting task {task_id}", "task_id": task_id})

        start = time.time()
        full_result = ""
        
        try:
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
            
            logger.info(f"✅ [EXECUTOR] Generation complete for {task_id}.")
        except Exception as e:
            logger.error(f"❌ [EXECUTOR] Task {task_id} FAILED: {e}")
            full_result = f"Error: {e}"
            await self.speak(Subject.LOG_ERROR, {"content": str(e), "task_id": task_id})

        elapsed = round(time.time() - start, 2)
        logger.info(f"🏁 [EXECUTOR] Task {task_id} finished in {elapsed}s.")

        # Publish final result
        result_payload = {
            "task_id": task_id,
            "status": "PASS" if full_result else "FAIL",
            "summary": full_result,
            "runtime": self.executor_runtime,
            "elapsed_sec": elapsed,
            "target_tool": payload.get("target_tool"),
        }

        await self.speak(Subject.TASK_EXEC_RESULT, result_payload)

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