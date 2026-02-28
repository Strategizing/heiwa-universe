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

class ExecutorAgent(BaseAgent):
    """
    Processes execution requests dispatched by the Planner/Messenger.
    """

    def __init__(self):
        super().__init__(name="heiwa-executor")
        self.executor_runtime = str(os.getenv("HEIWA_EXECUTOR_RUNTIME", "railway")).strip().lower() or "railway"
        self.engine = CognitionEngine()

    async def _handle_exec(self, data: dict[str, Any]) -> None:
        """Process a single execution request with streaming."""
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
        
        # Build a system prompt appropriate for the intent
        system = self._build_system_prompt(intent_class)

        logger.info(f"🚀 Starting execution for {task_id} using {target_model}")

        try:
            async for chunk in self.engine.generate_stream(instruction, target_model, system):
                full_result += chunk
                # Stream chunk as a thought for real-time UI feedback
                await self.speak(Subject.LOG_THOUGHT, {
                    "task_id": task_id,
                    "agent": self.name,
                    "content": chunk,
                    "stream": True
                })
        except Exception as e:
            logger.error(f"Execution failed: {e}")
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
    def _build_system_prompt(intent_class: str) -> str:
        """Return a focused system prompt for the given intent."""
        prompts = {
            "build": (
                "You are Heiwa, an expert software engineer. "
                "Write clean, production-ready code. "
                "Include brief comments explaining non-obvious decisions. "
                "Return the complete implementation."
            ),
            "research": (
                "You are Heiwa, a meticulous research analyst. "
                "Provide well-sourced, structured findings. "
                "Highlight confidence levels and uncertainties. "
                "Be concise but thorough."
            ),
            "deploy": (
                "You are Heiwa, a DevOps engineer. "
                "Provide exact commands and configs needed. "
                "Include health checks and rollback steps. "
                "Safety is the top priority."
            ),
            "operate": (
                "You are Heiwa, a systems operator. "
                "Diagnose issues methodically. "
                "Provide exact commands to run. "
                "Include verification steps."
            ),
            "automation": (
                "You are Heiwa, an automation specialist. "
                "Design reliable, idempotent workflows. "
                "Include error handling and logging. "
                "Prefer simple, maintainable solutions."
            ),
        }
        return prompts.get(
            intent_class,
            "You are Heiwa, a capable AI assistant. Be concise, accurate, and helpful.",
        )