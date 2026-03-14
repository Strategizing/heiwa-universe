"""
Heiwa Executor Agent

Listens on the local event bus for execution requests and processes
them through the tiered gateway. Runs on Railway for API-routed tasks.
Remote workers receive tasks via WebSocket, not this agent.
"""

import asyncio
import logging
import os
import sys
import time
from pathlib import Path
from typing import Any

from heiwa_hub.agents.base import BaseAgent
from heiwa_protocol.protocol import Subject
from heiwa_protocol.routing import BROKER_ENVELOPE_VERSION, BrokerRouteResult
from heiwa_sdk import HeiwaClawGateway
from heiwa_sdk.db import Database

logger = logging.getLogger("Executor")


class ExecutorAgent(BaseAgent):
    def __init__(self):
        super().__init__(name="heiwa-executor")
        self.executor_runtime = self._resolve_runtime()
        self.root = Path(__file__).resolve().parents[3]
        self.db = Database()
        self.gateway = HeiwaClawGateway(self.root)

    async def run(self):
        await self.start()

        await self.listen(Subject.TASK_EXEC, self._handle_exec)
        await self.listen(Subject.TASK_EXEC_REQUEST_CODE, self._handle_exec)
        await self.listen(Subject.TASK_EXEC_REQUEST_RESEARCH, self._handle_exec)

        logger.info("Executor active (%s). Awaiting directives...", self.executor_runtime)

        while self.running:
            await asyncio.sleep(1)

    async def _handle_exec(self, data: dict[str, Any]) -> None:
        payload = data.get("data", data)
        task_id = payload.get("task_id", "unknown")
        instruction = payload.get("instruction", "")
        intent_class = payload.get("intent_class", "general")
        target_runtime = payload.get("target_runtime", "railway").lower()
        target_model = payload.get("target_model", "")
        target_tool = str(payload.get("target_tool", "heiwa_claw")).strip().lower() or "heiwa_claw"

        if target_runtime not in {self.executor_runtime, "both", "any"}:
            logger.info("Skipping task %s: target=%s, local=%s", task_id, target_runtime, self.executor_runtime)
            return

        route = BrokerRouteResult.from_payload({
            "request_id": payload.get("request_id", f"exec-{task_id}"),
            "task_id": task_id,
            "envelope_version": payload.get("envelope_version", BROKER_ENVELOPE_VERSION),
            "raw_text": payload.get("raw_text", instruction),
            "source_surface": payload.get("source_surface", "executor"),
            "intent_class": intent_class,
            "risk_level": payload.get("risk_level", "low"),
            "privacy_level": payload.get("privacy_level"),
            "compute_class": payload.get("compute_class", 1),
            "assigned_worker": payload.get("assigned_worker", ""),
            "target_tool": target_tool,
            "target_model": target_model,
            "target_runtime": target_runtime,
            "target_tier": payload.get("target_tier", "tier1_local"),
            "requires_approval": payload.get("requires_approval", False),
            "rationale": payload.get("rationale", ""),
            "normalization": payload.get("normalization", {}),
        })
        dispatch = self.gateway.resolve(route)

        logger.info("Processing task: %s | Intent: %s", task_id, intent_class)
        await self.speak(Subject.TASK_STATUS, {
            "accepted": True,
            "reason": None,
            "task_id": task_id,
            "step_id": payload.get("step_id", "executor"),
            "status": "RUNNING",
            "message": f"Executor started task execution.",
            "runtime": self.executor_runtime,
            "response_channel_id": payload.get("response_channel_id"),
            "response_thread_id": payload.get("response_thread_id"),
        })

        start = time.time()
        full_result = ""
        exec_status = "FAIL"

        try:
            if dispatch.direct_execution and target_tool == "heiwa_ops" and intent_class == "audit":
                logger.info("Routing %s through bounded Class 1 audit.", task_id)
                exec_status, full_result = await self._run_bounded_audit(instruction)
            else:
                logger.info(
                    "Routing %s through HeiwaClaw (%s -> %s).",
                    task_id, dispatch.provider, dispatch.adapter_tool,
                )
                code, output = await self.gateway.execute(route, instruction)
                exec_status = "PASS" if code == 0 else "FAIL"
                full_result = output.strip()
        except Exception as e:
            logger.error("Task %s FAILED: %s", task_id, e)
            full_result = f"Error: {e}"
            exec_status = "FAIL"

        elapsed = round(time.time() - start, 2)
        logger.info("Task %s finished in %ss.", task_id, elapsed)

        result_payload = {
            "task_id": task_id,
            "status": exec_status,
            "summary": full_result,
            "runtime": self.executor_runtime,
            "elapsed_sec": elapsed,
            "target_tool": dispatch.gateway_tool,
            "adapter_tool": dispatch.adapter_tool,
            "provider": dispatch.provider,
            "target_model": route.target_model,
            "intent_class": intent_class,
            "requested_by": payload.get("requested_by"),
            "response_channel_id": payload.get("response_channel_id"),
            "response_thread_id": payload.get("response_thread_id"),
        }

        await self.speak(Subject.TASK_EXEC_RESULT, result_payload)
        await self.speak(Subject.TASK_STATUS, {
            "accepted": exec_status == "PASS",
            "reason": None if exec_status == "PASS" else "Executor produced a failure result.",
            "task_id": task_id,
            "step_id": payload.get("step_id", "executor"),
            "status": "DELIVERED",
            "message": f"Execution result published ({exec_status}).",
            "runtime": self.executor_runtime,
            "target_tool": dispatch.gateway_tool,
            "response_channel_id": payload.get("response_channel_id"),
            "response_thread_id": payload.get("response_thread_id"),
        })

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
            sys.executable, str(script),
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
