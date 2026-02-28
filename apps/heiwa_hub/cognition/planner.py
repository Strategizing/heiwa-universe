from __future__ import annotations

import json
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import Any

from heiwa_hub.cognition.intent_normalizer import (
    INTENT_ENUM,
    RISK_ENUM,
    RUNTIME_ENUM,
    TOOL_ENUM,
    IntentNormalizer,
    IntentProfile,
)
from heiwa_hub.cognition.llm_local import LLMPolicyError, LocalLLMEngine


@dataclass
class StepPlan:
    step_id: str
    title: str
    instruction: str
    subject: str
    target_runtime: str
    target_tool: str
    target_tier: str
    required_capability: str = ""


@dataclass
class TaskPlan:
    task_id: str
    parent_task_id: str
    plan_id: str
    intent_class: str
    risk_level: str
    requires_approval: bool
    raw_text: str
    requested_by: str
    source_channel_id: int | str
    source_message_id: int | str
    response_channel_id: int | str
    response_thread_id: int | str | None
    target_runtime: str
    target_tool: str
    target_tier: str
    steps: list[StepPlan]
    normalization: dict[str, Any] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        payload = asdict(self)
        payload["steps"] = [asdict(step) for step in self.steps]
        payload["step_id"] = payload["steps"][0]["step_id"] if payload["steps"] else "step-0"
        return payload


class LocalTaskPlanner:
    """Produces schema-like execution plans from free-form text."""

    def __init__(self) -> None:
        root = Path(__file__).resolve().parents[3]
        candidates = [
            root / "config" / "schemas" / "task_envelope_v2.schema.json",
            root / "schemas" / "task_envelope_v2.schema.json",
        ]
        self.schema_path = next((path for path in candidates if path.exists()), candidates[0])
        self.engine: LocalLLMEngine | None = None
        try:
            self.engine = LocalLLMEngine()
        except LLMPolicyError:
            # Keep planner running in deterministic fallback mode.
            self.engine = None
        self.normalizer = IntentNormalizer(engine=self.engine)

    def plan(
        self,
        task_id: str,
        raw_text: str,
        requested_by: str,
        source_channel_id: int | str,
        source_message_id: int | str,
        response_channel_id: int | str,
        response_thread_id: int | str | None,
        parent_task_id: str = "",
        intent_profile: IntentProfile | None = None,
    ) -> TaskPlan:
        profile = intent_profile or self.normalizer.normalize(raw_text)
        steps = self._build_steps(profile=profile, raw_text=raw_text)

        runtimes = {step.target_runtime for step in steps}
        tools = {step.target_tool for step in steps}
        tiers = {step.target_tier for step in steps}
        target_runtime = "both" if len(runtimes) > 1 else next(iter(runtimes), profile.preferred_runtime)
        target_tool = "ollama" if len(tools) > 1 else next(iter(tools), profile.preferred_tool)
        target_tier = next(iter(tiers), profile.preferred_tier)

        plan = TaskPlan(
            task_id=task_id,
            parent_task_id=parent_task_id,
            plan_id=f"plan-{task_id}",
            intent_class=profile.intent_class,
            risk_level=profile.risk_level,
            requires_approval=profile.requires_approval,
            raw_text=raw_text,
            requested_by=requested_by,
            source_channel_id=source_channel_id,
            source_message_id=source_message_id,
            response_channel_id=response_channel_id,
            response_thread_id=response_thread_id,
            target_runtime=target_runtime,
            target_tool=target_tool,
            target_tier=target_tier,
            steps=steps,
            normalization=profile.to_dict(),
        )
        self.validate_task_envelope(plan.to_dict())
        return plan

    def validate_task_envelope(self, payload: dict[str, Any]) -> None:
        required = {
            "task_id",
            "parent_task_id",
            "plan_id",
            "step_id",
            "raw_text",
            "intent_class",
            "risk_level",
            "requires_approval",
            "requested_by",
            "source_channel_id",
            "source_message_id",
            "response_channel_id",
            "response_thread_id",
            "target_runtime",
            "target_tool",
        }
        missing = required - payload.keys()
        if missing:
            raise ValueError(f"task envelope missing required fields: {sorted(missing)}")

        if payload["intent_class"] not in INTENT_ENUM:
            raise ValueError(f"invalid intent_class: {payload['intent_class']}")
        if payload["risk_level"] not in RISK_ENUM:
            raise ValueError(f"invalid risk_level: {payload['risk_level']}")
        if payload["target_runtime"] not in RUNTIME_ENUM:
            raise ValueError(f"invalid target_runtime: {payload['target_runtime']}")
        # Tool validation skipped for dynamic tool mesh localization

    def _build_steps(self, profile: IntentProfile, raw_text: str) -> list[StepPlan]:
        now = int(time.time() * 1000)
        step_num = 0
        instruction = profile.normalized_instruction
        steps: list[StepPlan] = []

        from heiwa_protocol.protocol import Subject

        def next_step_id() -> str:
            nonlocal step_num
            step_num += 1
            return f"step-{now}-{step_num}"

        intent = profile.intent_class

        # --- Mesh & Management: High-level 'Magic' actions ---
        if intent == "status_check":
            steps.append(
                StepPlan(
                    step_id=next_step_id(),
                    title="Mesh Pulse Diagnostic",
                    instruction="Analyze the current mesh health and report status.",
                    subject=Subject.TASK_EXEC.value,
                    target_runtime="railway",
                    target_tool="heiwa_ops",
                    target_tier="tier1_local",
                )
            )
            return steps

        if intent == "mesh_ops":
            steps.append(
                StepPlan(
                    step_id=next_step_id(),
                    title="Sovereign Mesh Audit",
                    instruction=instruction,
                    subject=Subject.TASK_EXEC.value,
                    target_runtime="macbook",
                    target_tool="heiwa_ops",
                    target_tier="tier3_orchestrator",
                )
            )
            return steps

        if intent == "self_buff":
            steps.append(
                StepPlan(
                    step_id=next_step_id(),
                    title="Native Self-Improvement Sequence",
                    instruction=instruction,
                    subject=Subject.TASK_EXEC.value,
                    target_runtime="macbook",
                    target_tool="heiwa_buff",
                    target_tier="tier5_heavy_code",
                )
            )
            return steps

        # --- Tasks that need shell/file access: dispatch to nodes ---
        if intent == "build":
            steps.append(
                StepPlan(
                    step_id=next_step_id(),
                    title="Implement code changes",
                    instruction=instruction,
                    subject=Subject.TASK_EXEC.value,
                    target_runtime=profile.preferred_runtime,
                    target_tool="heiwa_code",
                    target_tier=profile.preferred_tier,
                    required_capability="mcp.code_generation",
                )
            )
            return steps

        if intent == "research":
            steps.append(
                StepPlan(
                    step_id=next_step_id(),
                    title="Gather and synthesize findings",
                    instruction=instruction,
                    subject=Subject.TASK_EXEC.value,
                    target_runtime="railway",
                    target_tool="heiwa_claw",
                    target_tier=profile.preferred_tier,
                    required_capability="mcp.research",
                )
            )
            return steps

        # --- Default: general orchestration via Railway LLM ---
        steps.append(
            StepPlan(
                step_id=next_step_id(),
                title="General orchestration response",
                instruction=instruction,
                subject=Subject.TASK_EXEC.value,
                target_runtime="railway",
                target_tool="heiwa_reflex",
                target_tier=profile.preferred_tier,
                required_capability="mcp.strategy",
            )
        )
        return steps
