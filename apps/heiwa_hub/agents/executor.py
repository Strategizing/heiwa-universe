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
        
        # Listen to all execution variants
        await self.listen(Subject.TASK_EXEC_REQUEST_CODE, self._handle_exec)
        await self.listen(Subject.TASK_EXEC_REQUEST_RESEARCH, self._handle_exec)
        await self.listen(Subject.TASK_EXEC_REQUEST_AUTOMATION, self._handle_exec)
        await self.listen(Subject.TASK_EXEC_REQUEST_OPERATE, self._handle_exec)
        
        logger.info(f"🦾 Executor Active ({self.executor_runtime}). Awaiting directives...")
        
        while self.running:
            await asyncio.sleep(1)

    async def _handle_exec(self, data: dict[str, Any]) -> None:
        """Process a single execution request with reasoning and conscience."""
        payload = data.get("data", data)
        task_id = payload.get("task_id", "unknown")
        step_id = payload.get("step_id", "unknown")
        instruction = payload.get("instruction", "")
        intent_class = payload.get("intent_class", "general")
        target_runtime = payload.get("target_runtime", "railway")
        target_model = payload.get("target_model", "google/gemini-2.0-flash")

        if target_runtime not in {self.executor_runtime, "both"}:
            return

        start = time.time()
        full_result = ""
        
        # Build an augmentation prompt for the intent
        system_augmentation = self._build_intent_context(intent_class)

        logger.info(f"🚀 [SOTA COGNITION] Task: {task_id} | Engine: evaluate_and_stream")

        try:
            # Use evaluate_and_stream for reasoning + conscience + execution
            async for chunk in self.engine.evaluate_and_stream(
                origin=self.name,
                intent=intent_class,
                instruction=instruction,
                model=target_model,
                system=system_augmentation
            ):
                full_result += chunk
                # Stream chunk as a thought for real-time UI feedback
                await self.speak(Subject.LOG_THOUGHT, {
                    "task_id": task_id,
                    "agent": self.name,
                    "content": chunk,
                    "stream": True
                })
        except Exception as e:
            logger.error(f"❌ Execution failed: {e}")
            full_result = f"Error: {e}"

        elapsed = round(time.time() - start, 2)

        # Publish final result
        result_payload = {
            "task_id": task_id,
            "step_id": step_id,
            "status": "PASS" if full_result else "FAIL",
            "summary": full_result,
            "runtime": self.executor_runtime,
            "executor_id": self.name,
            "intent_class": intent_class,
            "elapsed_sec": elapsed,
            "target_tool": payload.get("target_tool"),
            "response_channel_id": payload.get("response_channel_id"),
            "response_thread_id": payload.get("response_thread_id"),
        }

        await self.speak(Subject.TASK_EXEC_RESULT, result_payload)
        logger.info(f"✅ Exec complete: task={task_id} elapsed={elapsed}s")

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