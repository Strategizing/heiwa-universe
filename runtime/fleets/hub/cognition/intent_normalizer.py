from __future__ import annotations

from dataclasses import dataclass
import re
from typing import Any

from fleets.hub.cognition.llm_local import LocalLLMEngine

INTENT_ENUM = {
    "build",
    "research",
    "deploy",
    "operate",
    "automation",
    "files",
    "notion",
    "discord",
    "chat",
    "general",
}
RISK_ENUM = {"low", "medium", "high"}
RUNTIME_ENUM = {"railway", "macbook", "both"}
TOOL_ENUM = {"codex", "openclaw", "picoclaw", "n8n", "ollama"}
TIER_ENUM = {
    "tier1_local",
    "tier2_fast_context",
    "tier3_orchestrator",
    "tier4_pooled_orchestrator",
    "tier5_heavy_code",
    "tier6_premium_context",
    "tier7_supreme_court",
}

_INTENT_RULES = (
    ("chat", ("hi", "hello", "hey", "wsg", "sup", "ping", "test", "wake", "status"), "low", False),
    ("automation", ("automate", "workflow", "schedule", "cron", "n8n"), "high", True),
    ("research", ("research", "analyze", "compare", "summarize", "investigate"), "low", False),
    ("build", ("build", "create", "implement", "code", "script", "project"), "medium", False),
    ("deploy", ("deploy", "release", "ship", "publish", "production"), "high", True),
    ("operate", ("fix", "debug", "incident", "monitor", "health"), "high", True),
    ("files", ("file", "move", "rename", "delete", "folder"), "high", True),
    ("notion", ("notion", "wiki", "docs", "database", "meeting notes"), "medium", True),
    ("discord", ("discord", "channel", "dm", "message", "announce"), "high", True),
)

_INTENT_DEFAULTS = {
    "build": ("macbook", "codex", "tier5_heavy_code"),
    "research": ("both", "openclaw", "tier2_fast_context"),
    "deploy": ("macbook", "codex", "tier5_heavy_code"),
    "operate": ("macbook", "codex", "tier5_heavy_code"),
    "automation": ("macbook", "n8n", "tier3_orchestrator"),
    "files": ("macbook", "codex", "tier5_heavy_code"),
    "notion": ("macbook", "codex", "tier3_orchestrator"),
    "discord": ("macbook", "codex", "tier3_orchestrator"),
    "chat": ("railway", "ollama", "tier1_local"),
    "general": ("railway", "ollama", "tier1_local"),
}


@dataclass
class IntentProfile:
    intent_class: str
    risk_level: str
    requires_approval: bool
    preferred_runtime: str
    preferred_tool: str
    preferred_tier: str
    normalized_instruction: str
    assumptions: list[str]
    missing_details: list[str]
    confidence: float
    underspecified: bool

    def to_dict(self) -> dict[str, Any]:
        return {
            "intent_class": self.intent_class,
            "risk_level": self.risk_level,
            "requires_approval": self.requires_approval,
            "preferred_runtime": self.preferred_runtime,
            "preferred_tool": self.preferred_tool,
            "preferred_tier": self.preferred_tier,
            "normalized_instruction": self.normalized_instruction,
            "assumptions": self.assumptions,
            "missing_details": self.missing_details,
            "confidence": self.confidence,
            "underspecified": self.underspecified,
        }


class IntentNormalizer:
    """
    Converts vague natural-language requests into a consistent intent profile
    and a structured execution brief.
    """

    def __init__(self, engine: LocalLLMEngine | None = None) -> None:
        self.engine = engine

    def normalize(self, raw_text: str) -> IntentProfile:
        text = " ".join((raw_text or "").split()).strip()
        if not text:
            text = "handle this task with safe defaults"

        # Phase 4 Optimization: Regex Triage First (Zero Cost)
        inferred = self._infer_with_rules(text)
        intent = inferred["intent_class"]
        
        # Only burn API quota if the regex wall fails to classify it (general)
        if intent == "general":
            llm = self._infer_with_llm(text)
            if llm:
                intent = llm["intent_class"]
                risk = llm["risk_level"]
                requires_approval = llm["requires_approval"]
                runtime = llm["preferred_runtime"]
                tool = llm["preferred_tool"]
                tier = llm.get("preferred_tier", _INTENT_DEFAULTS.get(intent, ("railway", "ollama", "tier1_local"))[2])
                confidence = llm["confidence"]
            else:
                risk = inferred["risk_level"]
                requires_approval = inferred["requires_approval"]
                runtime, tool, tier = _INTENT_DEFAULTS[intent]
                confidence = 0.50
        else:
            risk = inferred["risk_level"]
            requires_approval = inferred["requires_approval"]
            runtime, tool, tier = _INTENT_DEFAULTS[intent]
            confidence = 0.95 # High confidence for explicit regex matches

        missing = self._missing_details(text, intent)
        underspecified = len(missing) >= 2 or len(text.split()) < 8
        assumptions = self._assumptions(intent, text, missing)
        normalized_instruction = self._structured_instruction(
            original=text,
            intent=intent,
            runtime=runtime,
            tool=tool,
            tier=tier,
            assumptions=assumptions,
            missing=missing,
        )

        return IntentProfile(
            intent_class=intent,
            risk_level=risk,
            requires_approval=requires_approval,
            preferred_runtime=runtime,
            preferred_tool=tool,
            preferred_tier=tier,
            normalized_instruction=normalized_instruction,
            assumptions=assumptions,
            missing_details=missing,
            confidence=confidence,
            underspecified=underspecified,
        )

    def _infer_with_rules(self, text: str) -> dict[str, Any]:
        lowered = text.lower()
        for intent, keywords, risk, approval in _INTENT_RULES:
            if any(self._keyword_match(lowered, word) for word in keywords):
                return {
                    "intent_class": intent,
                    "risk_level": risk,
                    "requires_approval": approval,
                }
        return {
            "intent_class": "general",
            "risk_level": "low",
            "requires_approval": False,
        }

    @staticmethod
    def _keyword_match(lowered_text: str, keyword: str) -> bool:
        token = str(keyword or "").strip().lower()
        if not token:
            return False
        if " " in token:
            # Phrase match with non-word boundaries so "meeting notes" works while
            # avoiding substring collisions inside larger words.
            pattern = rf"(?<!\w){re.escape(token)}(?!\w)"
            return re.search(pattern, lowered_text) is not None
        pattern = rf"\b{re.escape(token)}\b"
        return re.search(pattern, lowered_text) is not None

    def _infer_with_llm(self, text: str) -> dict[str, Any]:
        if not self.engine or not self.engine.is_available("railway"):
            return {}

        prompt = (
            "Classify this request and return JSON only with keys: "
            "intent_class, risk_level, requires_approval, preferred_runtime, preferred_tool, confidence.\n"
            "Enums:\n"
            "- intent_class: build,research,deploy,operate,automation,files,notion,discord,general\n"
            "- risk_level: low,medium,high\n"
            "- preferred_runtime: railway,macbook,both\n"
            "- preferred_tool: codex,openclaw,picoclaw,n8n,ollama\n"
            "- confidence: 0..1\n"
            "Request:\n"
            f"{text}"
        )
        data = self.engine.generate_json(prompt=prompt, runtime="railway", complexity="medium")
        intent = str(data.get("intent_class", "")).strip().lower()
        risk = str(data.get("risk_level", "")).strip().lower()
        runtime = str(data.get("preferred_runtime", "")).strip().lower()
        tool = str(data.get("preferred_tool", "")).strip().lower()
        approval = data.get("requires_approval")
        confidence_raw = data.get("confidence", 0.0)

        try:
            confidence = float(confidence_raw)
        except (TypeError, ValueError):
            confidence = 0.0

        if (
            intent in INTENT_ENUM
            and risk in RISK_ENUM
            and runtime in RUNTIME_ENUM
            and tool in TOOL_ENUM
            and isinstance(approval, bool)
        ):
            return {
                "intent_class": intent,
                "risk_level": risk,
                "requires_approval": approval,
                "preferred_runtime": runtime,
                "preferred_tool": tool,
                "confidence": max(0.0, min(1.0, confidence)),
            }
        return {}

    def _missing_details(self, text: str, intent: str) -> list[str]:
        lowered = text.lower()
        out: list[str] = []

        if len(text.split()) < 8:
            out.append("primary objective and desired outcome")

        if intent == "build":
            if not any(k in lowered for k in ("python", "typescript", "javascript", "go", "rust", "java")):
                out.append("preferred language or framework")
            if not any(k in lowered for k in ("file", "api", "service", "cli", "script", "app")):
                out.append("expected output artifact")
        elif intent == "research":
            if not any(k in lowered for k in ("today", "latest", "recent", "202", "this week", "this month")):
                out.append("time horizon / recency window")
            if not any(k in lowered for k in ("compare", "best", "tradeoff", "criteria", "pros", "cons")):
                out.append("evaluation criteria")
        elif intent == "deploy":
            if not any(k in lowered for k in ("railway", "production", "staging", "service", "environment")):
                out.append("target environment or service")
            if not any(k in lowered for k in ("rollback", "safe", "health", "downtime")):
                out.append("rollback and safety requirements")
        elif intent == "operate":
            if not any(k in lowered for k in ("service", "api", "worker", "database", "nats", "queue", "agent")):
                out.append("affected system/component")
        elif intent == "automation":
            if not any(k in lowered for k in ("hourly", "daily", "weekly", "schedule", "trigger", "event")):
                out.append("trigger cadence or event source")
            if not any(k in lowered for k in ("discord", "notion", "email", "slack", "webhook", "output")):
                out.append("destination for automation outputs")

        return out

    def _assumptions(self, intent: str, text: str, missing: list[str]) -> list[str]:
        assumptions = [
            "Use local-first execution when possible and keep cloud actions minimal.",
            "Do not run destructive actions without explicit approval.",
            "Prefer incremental, reversible changes and report exact commands used.",
        ]
        if intent == "research":
            assumptions.append("Cite primary sources and highlight confidence/uncertainty.")
        if intent in {"deploy", "operate"}:
            assumptions.append("Include health checks and rollback notes in outputs.")
        if missing:
            assumptions.append("Proceed with best-effort defaults for missing details and surface them clearly.")
        return assumptions

    def _structured_instruction(
        self,
        original: str,
        intent: str,
        runtime: str,
        tool: str,
        tier: str,
        assumptions: list[str],
        missing: list[str],
    ) -> str:
        lines = [
            "Original Request:",
            original,
            "",
            "Execution Brief:",
            f"- Intent class: {intent}",
            f"- Preferred runtime: {runtime}",
            f"- Preferred tool: {tool}",
            f"- Target tier: {tier}",
            "",
            "Assumptions:",
        ]
        lines.extend([f"- {item}" for item in assumptions])
        if missing:
            lines.extend(["", "Missing Details Detected:"])
            lines.extend([f"- {item}" for item in missing])
        lines.extend(
            [
                "",
                "Instruction:",
                "Execute the request using the brief above. If details are missing, continue with safe defaults and report assumptions in the result.",
            ]
        )
        return "\n".join(lines)
