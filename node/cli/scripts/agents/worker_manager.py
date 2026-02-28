#!/usr/bin/env python3
from __future__ import annotations

import asyncio
import json
import logging
import os
import sys
import time
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[4]
RUNTIME_ROOT = ROOT / "runtime"
if str(RUNTIME_ROOT) not in sys.path:
    sys.path.insert(0, str(RUNTIME_ROOT))

from fleets.hub.agents.base import BaseAgent
from fleets.hub.protocol import Subject

logger = logging.getLogger("WorkerManager")


class WorkerManager(BaseAgent):
    """Macbook execution daemon for codex/openclaw/picoclaw/n8n task steps."""

    def __init__(self) -> None:
        use_remote_nats = os.getenv("HEIWA_USE_REMOTE_NATS", "0") == "1"
        if not use_remote_nats:
            current_nats = os.getenv("NATS_URL", "")
            if not current_nats or "railway.internal" in current_nats:
                os.environ["NATS_URL"] = os.getenv("HEIWA_LOCAL_NATS_URL", "nats://127.0.0.1:4222")

        super().__init__(name="heiwa-worker-manager")
        self.root = ROOT
        self.node_id = os.getenv("HEIWA_NODE_ID", "macbook")
        self.node_type = os.getenv("HEIWA_NODE_TYPE", "mobile_node") # mobile_node | heavy_compute
        self.capabilities = {
            item.strip().lower()
            for item in os.getenv("HEIWA_CAPABILITIES", "").split(",")
            if item.strip()
        }
        # Add default capabilities based on node type if none provided
        if not self.capabilities:
            if self.node_type == "heavy_compute":
                self.capabilities = {"heavy_compute", "gpu_native", "standard_compute"}
            else:
                self.capabilities = {"standard_compute", "workspace_interaction", "agile_coding"}

        self.warm_ttl_sec = int(os.getenv("HEIWA_WORKER_WARM_TTL_SEC", "600"))
        self.concurrency = int(os.getenv("HEIWA_EXECUTOR_CONCURRENCY", "2"))
        self.llm_mode = os.getenv("HEIWA_LLM_MODE", "local_only").lower()
        self.allowed_outbound_targets = {
            item.strip()
            for item in os.getenv("HEIWA_ALLOWED_OUTBOUND_TARGETS", "").split(",")
            if item.strip()
        }
        self.last_used: dict[str, float] = {}
        self.sem = asyncio.Semaphore(max(1, self.concurrency))

        wrappers_root = self.root / "node" / "cli" / "scripts" / "agents" / "wrappers"
        self.codex_wrapper = wrappers_root / "codex_exec.sh"
        self.openclaw_wrapper = wrappers_root / "openclaw_exec.sh"
        self.picoclaw_wrapper = wrappers_root / "picoclaw_exec.py"
        self.ollama_wrapper = wrappers_root / "ollama_exec.py"

        self.subject_tools = {
            Subject.TASK_EXEC_REQUEST_CODE.value: "codex",
            Subject.TASK_EXEC_REQUEST_RESEARCH.value: "openclaw",
            Subject.TASK_EXEC_REQUEST_AUTOMATION.value: "n8n",
            Subject.TASK_EXEC_REQUEST_OPERATE.value: "codex",
        }

    @staticmethod
    def _unwrap(data: dict[str, Any]) -> dict[str, Any]:
        maybe = data.get("data")
        if isinstance(maybe, dict):
            return maybe
        return data

    async def run(self):
        await self.connect()

        if self.llm_mode != "local_only":
            logger.error("HEIWA_LLM_MODE must be local_only. Current=%s", self.llm_mode)
            return

        if not self.nc:
            logger.error("NATS is required for worker_manager")
            return

        await self.nc.subscribe(Subject.TASK_EXEC_REQUEST_CODE.value, cb=self._handle_msg)
        await self.nc.subscribe(Subject.TASK_EXEC_REQUEST_RESEARCH.value, cb=self._handle_msg)
        await self.nc.subscribe(Subject.TASK_EXEC_REQUEST_AUTOMATION.value, cb=self._handle_msg)
        await self.nc.subscribe(Subject.TASK_EXEC_REQUEST_OPERATE.value, cb=self._handle_msg)

        logger.info(
            "worker_manager active. subjects=%s concurrency=%s warm_ttl=%ss",
            list(self.subject_tools.keys()),
            self.concurrency,
            self.warm_ttl_sec,
        )

        while True:
            await asyncio.sleep(1)

    async def _handle_msg(self, msg):
        data = json.loads(msg.data.decode())
        payload = self._unwrap(data)
        payload.setdefault("target_tool", self.subject_tools.get(msg.subject, "ollama"))
        payload.setdefault("subject", msg.subject)
        await self.execute(payload)

    async def execute(self, payload: dict[str, Any]) -> None:
        async with self.sem:
            start = time.time()
            task_id = str(payload.get("task_id", "unknown"))
            step_id = str(payload.get("step_id", f"step-{int(start*1000)}"))
            runtime = str(payload.get("target_runtime", "macbook"))
            tool = str(payload.get("target_tool", "ollama")).lower()
            intent_class = str(payload.get("intent_class", "general")).lower()
            instruction = str(payload.get("instruction") or payload.get("raw_text") or "").strip()

            # Dynamic Identity Routing & Prompt Enrichment
            try:
                ops_dir = self.root / "node" / "cli" / "scripts" / "ops"
                if str(ops_dir) not in sys.path:
                    sys.path.append(str(ops_dir))
                from identity_selector import load_profiles, select_identity
                
                profiles = load_profiles()
                selection = select_identity(intent_class, profiles)
                selected = selection.get("selected", {})
                
                desc = selected.get("description", "Heiwa operator.")
                # Enrich the prompt with the identity role and executive summary requirement
                instruction = (
                    f"[ROLE]: {desc}\n"
                    "Requirement: ALWAYS include a '## EXECUTIVE SUMMARY' in polished markdown "
                    "at the start of your response. This is for human oversight. "
                    "Follow it with '---' then any technical details or code.\n\n"
                    f"{instruction}"
                )
                
                # Assign specific models if defined in identity
                models = selected.get("models", {})
                payload["report_channel"] = selected.get("actions", {}).get("report_channel")
                if tool == "openclaw" and models.get("openclaw"):
                    # Use the preferred model (e.g. qwen/deepseek)
                    os.environ["OPENCLAW_MODEL"] = models["openclaw"][0]
                elif tool == "ollama" and models.get("openclaw"):
                    # Fallback mapping: use openclaw's model for ollama if applicable
                    os.environ["HEIWA_OLLAMA_MODEL"] = models["openclaw"][0].split("/")[-1]
                elif tool == "picoclaw" and models.get("picoclaw"):
                    os.environ["PICOCLAW_MODEL"] = models["picoclaw"][0]
            except Exception as e:
                logger.warning("Failed to resolve identity, using defaults. Error: %s", e)

            required_caps = set(payload.get("required_capabilities") or [])
            
            can_execute = False
            if runtime in {self.node_id, "both", "any", "all"}:
                can_execute = True
            elif required_caps and required_caps.issubset(self.capabilities):
                can_execute = True

            if not can_execute:
                await self._emit_result(
                    task_id=task_id,
                    step_id=step_id,
                    status="BLOCKED",
                    summary=f"Skipped on {self.node_id} (target_runtime={runtime}, required_caps={required_caps})",
                    duration_ms=int((time.time() - start) * 1000),
                    runtime=self.node_id,
                    error=None,
                    artifacts=[{"kind": "dispatch", "value": "capability_mismatch"}],
                    payload=payload,
                )
                return

            if intent_class in {"discord", "notion", "deploy", "automation"} and not self.allowed_outbound_targets:
                await self._emit_result(
                    task_id=task_id,
                    step_id=step_id,
                    status="BLOCKED",
                    summary="Outbound action blocked: HEIWA_ALLOWED_OUTBOUND_TARGETS is empty",
                    duration_ms=int((time.time() - start) * 1000),
                    runtime="macbook",
                    error="outbound_allowlist_empty",
                    artifacts=[],
                    payload=payload,
                )
                return

            if tool in {"openclaw", "picoclaw", "ollama"} and self.llm_mode != "local_only":
                await self._emit_result(
                    task_id=task_id,
                    step_id=step_id,
                    status="BLOCKED",
                    summary="Local-model-only policy blocked non-local execution",
                    duration_ms=int((time.time() - start) * 1000),
                    runtime="macbook",
                    error="llm_mode_violation",
                    artifacts=[],
                    payload=payload,
                )
                return

            warm_hit = self._is_warm(tool)
            await self.speak(
                Subject.TASK_STATUS,
                {
                    "task_id": task_id,
                    "step_id": step_id,
                    "status": "RUNNING",
                    "runtime": "macbook",
                    "tool": tool,
                    "warm_hit": warm_hit,
                },
            )

            status = "PASS"
            summary = ""
            error = None
            artifacts: list[dict[str, str]] = []

            try:
                if tool == "codex":
                    code, out = await self._run_codex(instruction)
                    status = "PASS" if code == 0 else "FAIL"
                    summary = out[:1800] or "Codex execution completed."
                    artifacts.append({"kind": "stdout", "value": out[:500]})
                elif tool == "openclaw":
                    code, out = await self._run_openclaw(instruction)
                    status = "PASS" if code == 0 else "FAIL"
                    summary = out[:1800] or "OpenClaw execution completed."
                    artifacts.append({"kind": "stdout", "value": out[:500]})
                elif tool == "picoclaw":
                    code, out = await self._run_picoclaw(instruction)
                    status = "PASS" if code == 0 else "FAIL"
                    summary = out[:1800] or "PicoClaw execution completed."
                    artifacts.append({"kind": "stdout", "value": out[:500]})
                elif tool == "ollama":
                    code, out = await self._run_ollama_persona(tool, instruction)
                    status = "PASS" if code == 0 else "FAIL"
                    summary = out[:1800] or f"{tool} local inference completed."
                    artifacts.append({"kind": "model", "value": tool})
                elif tool == "n8n":
                    status = "BLOCKED"
                    summary = "n8n executor is not installed/configured on this node"
                    error = "n8n_missing"
                else:
                    status = "BLOCKED"
                    summary = f"Unsupported target_tool: {tool}"
                    error = "unsupported_tool"
            except Exception as exc:
                status = "FAIL"
                summary = "Executor raised an exception"
                error = str(exc)

            self.last_used[tool] = time.time()

            await self._emit_result(
                task_id=task_id,
                step_id=step_id,
                status=status,
                summary=summary,
                duration_ms=int((time.time() - start) * 1000),
                runtime="macbook",
                error=error,
                artifacts=artifacts,
                payload=payload,
            )

    def _is_warm(self, tool: str) -> bool:
        ts = self.last_used.get(tool)
        if ts is None:
            return False
        return (time.time() - ts) <= self.warm_ttl_sec

    async def _run_codex(self, instruction: str) -> tuple[int, str]:
        if not self.codex_wrapper.exists():
            return 2, f"missing wrapper: {self.codex_wrapper}"

        return await self._run_shell_wrapper(self.codex_wrapper, instruction)

    async def _run_openclaw(self, instruction: str) -> tuple[int, str]:
        if not self.openclaw_wrapper.exists():
            return 2, f"missing wrapper: {self.openclaw_wrapper}"

        return await self._run_shell_wrapper(self.openclaw_wrapper, instruction)

    async def _run_picoclaw(self, instruction: str) -> tuple[int, str]:
        if not self.picoclaw_wrapper.exists():
            return 2, f"missing wrapper: {self.picoclaw_wrapper}"

        proc = await asyncio.create_subprocess_exec(
            sys.executable,
            str(self.picoclaw_wrapper),
            instruction,
            cwd=str(self.root),
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        stdout, stderr = await proc.communicate()
        output = (stdout or b"").decode(errors="ignore")
        err = (stderr or b"").decode(errors="ignore")
        combined = "\n".join(part for part in [output.strip(), err.strip()] if part)
        return proc.returncode, combined[:6000]

    async def _run_ollama_persona(self, tool: str, instruction: str) -> tuple[int, str]:
        if not self.ollama_wrapper.exists():
            return 2, f"missing wrapper: {self.ollama_wrapper}"

        prompt = (
            f"You are {tool} running in local-only mode. "
            f"Produce actionable output for this task:\n{instruction}"
        )

        proc = await asyncio.create_subprocess_exec(
            sys.executable,
            str(self.ollama_wrapper),
            prompt,
            cwd=str(self.root),
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        stdout, stderr = await proc.communicate()
        output = (stdout or b"").decode(errors="ignore")
        err = (stderr or b"").decode(errors="ignore")
        combined = "\n".join(part for part in [output.strip(), err.strip()] if part)
        return proc.returncode, combined[:6000]

    async def _run_shell_wrapper(self, wrapper: Path, instruction: str) -> tuple[int, str]:
        proc = await asyncio.create_subprocess_exec(
            str(wrapper),
            instruction,
            cwd=str(self.root),
            stdout=asyncio.subprocess.PIPE,
            stderr=asyncio.subprocess.PIPE,
        )
        stdout, stderr = await proc.communicate()
        output = (stdout or b"").decode(errors="ignore")
        err = (stderr or b"").decode(errors="ignore")
        combined = "\n".join(part for part in [output.strip(), err.strip()] if part)
        return proc.returncode, combined[:6000]

    async def _emit_result(
        self,
        task_id: str,
        step_id: str,
        status: str,
        summary: str,
        duration_ms: int,
        runtime: str,
        error: str | None,
        artifacts: list[dict[str, str]],
        payload: dict[str, Any],
    ) -> None:
        result = {
            "task_id": task_id,
            "step_id": step_id,
            "status": status,
            "summary": summary,
            "artifacts": artifacts,
            "logs_ref": None,
            "next_actions": [],
            "executor_id": self.name,
            "runtime": runtime,
            "duration_ms": duration_ms,
            "error": error,
            "response_channel_id": payload.get("response_channel_id"),
            "response_thread_id": payload.get("response_thread_id"),
            "report_channel": payload.get("report_channel"),
            "target_tool": payload.get("target_tool"),
            "target_runtime": payload.get("target_runtime"),
            "plan_id": payload.get("plan_id"),
        }
        await self.speak(Subject.TASK_EXEC_RESULT, result)
        await self.speak(
            Subject.TASK_STATUS,
            {
                "task_id": task_id,
                "step_id": step_id,
                "status": status,
                "runtime": runtime,
                "tool": payload.get("target_tool"),
                "duration_ms": duration_ms,
            },
        )


async def main() -> None:
    logging.basicConfig(level=logging.INFO)
    worker = WorkerManager()
    await worker.run()


if __name__ == "__main__":
    asyncio.run(main())
