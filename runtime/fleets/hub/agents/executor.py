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

from fleets.hub.agents.base import BaseAgent
from fleets.hub.cognition.llm_local import LocalLLMEngine
from fleets.hub.protocol import Subject

logger = logging.getLogger("Executor")

# Map intent complexity to LLM tier
_INTENT_COMPLEXITY = {
    "chat": "low",
    "general": "low",
    "research": "medium",
    "build": "high",
    "deploy": "high",
    "operate": "high",
    "automation": "medium",
    "files": "medium",
    "notion": "low",
    "discord": "low",
}


class ExecutorAgent(BaseAgent):
    """
    Processes execution requests dispatched by the Planner/Messenger.

    Listens on:
      - heiwa.tasks.exec.request.code
      - heiwa.tasks.exec.request.research
      - heiwa.tasks.exec.request.automation
      - heiwa.tasks.exec.request.operate
    """

    def __init__(self):
        super().__init__(name="heiwa-executor")
        self.executor_runtime = str(os.getenv("HEIWA_EXECUTOR_RUNTIME", "railway")).strip().lower() or "railway"
        self.engine: LocalLLMEngine | None = None
        try:
            self.engine = LocalLLMEngine()
        except Exception as e:
            logger.warning("LLM Engine unavailable: %s", e)

    async def run(self):
        try:
            await self.connect()
        except Exception:
            logger.warning("⚠️ NATS unavailable. Executor in standalone mode.")

        # Subscribe to all execution request subjects
        await self.listen(Subject.TASK_EXEC_REQUEST_CODE, self._handle_exec)
        await self.listen(Subject.TASK_EXEC_REQUEST_RESEARCH, self._handle_exec)
        await self.listen(Subject.TASK_EXEC_REQUEST_AUTOMATION, self._handle_exec)
        await self.listen(Subject.TASK_EXEC_REQUEST_OPERATE, self._handle_exec)

        logger.info("⚡ Executor online. Listening for work...")

        while self.running:
            await asyncio.sleep(1)

    async def _handle_exec(self, data: dict[str, Any]) -> None:
        """Process a single execution request."""
        payload = data.get("data", data)
        task_id = payload.get("task_id", "unknown")
        step_id = payload.get("step_id", "unknown")
        instruction = payload.get("instruction", "")
        intent_class = payload.get("intent_class", "general")
        target_runtime = payload.get("target_runtime", "railway")

        logger.info(
            "📥 Exec request: task=%s step=%s intent=%s runtime=%s",
            task_id, step_id, intent_class, target_runtime,
        )

        if target_runtime not in {self.executor_runtime, "both"}:
            logger.info(
                "⏭️ Skipping task=%s step=%s on %s executor (target_runtime=%s)",
                task_id,
                step_id,
                self.executor_runtime,
                target_runtime,
            )
            await self.speak(
                Subject.TASK_STATUS,
                {
                    "task_id": task_id,
                    "step_id": step_id,
                    "status": "DEFERRED",
                    "message": (
                        f"Executor {self.executor_runtime} deferred step; "
                        f"waiting for target_runtime={target_runtime} node."
                    ),
                    "runtime": self.executor_runtime,
                    "response_channel_id": payload.get("response_channel_id"),
                    "response_thread_id": payload.get("response_thread_id"),
                },
            )
            return

        start = time.time()
        result = await self._execute(instruction, intent_class, runtime=self.executor_runtime)
        elapsed = round(time.time() - start, 2)

        # Publish result
        result_payload = {
            "task_id": task_id,
            "step_id": step_id,
            "status": "PASS" if result else "FAIL",
            "summary": result[:2000] if result else "No output produced.",
            "runtime": self.executor_runtime,
            "executor_id": self.name,
            "intent_class": intent_class,
            "elapsed_sec": elapsed,
            "target_tool": payload.get("target_tool"),
            "response_channel_id": payload.get("response_channel_id"),
            "response_thread_id": payload.get("response_thread_id"),
            "artifacts": [],
        }

        await self.speak(Subject.TASK_EXEC_RESULT, result_payload)
        logger.info(
            "✅ Exec complete: task=%s elapsed=%.1fs provider=%s",
            task_id, elapsed, "llm",
        )

    async def _execute(self, instruction: str, intent_class: str, runtime: str = "auto") -> str:
        """Run the instruction through the LLM engine."""
        if not self.engine:
            return "(Executor has no LLM engine available)"

        complexity = _INTENT_COMPLEXITY.get(intent_class, "medium")

        # Build a system prompt appropriate for the intent
        system = self._build_system_prompt(intent_class)

        try:
            return self.engine.generate(
                prompt=instruction,
                complexity=complexity,
                system=system,
                runtime=runtime,
            )
        except Exception as e:
            logger.error("Execution failed: %s", e)
            return f"Error: {e}"

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
