"""
Heiwa Risk Scorer v1 — Rule-Based

Phase B Action 4. Assigns risk_level and requires_approval based on:
1. Intent class defaults (from the intent taxonomy)
2. Keyword escalators (dangerous verbs/nouns bump risk up)
3. Surface modifiers (Discord commands are lower-trust than CLI)

Design contract: broker_extraction_design_review.md Section 1.

Output: {"risk_level": "low"|"medium"|"high"|"critical", "requires_approval": bool, "escalation_reasons": list[str]}
"""
from __future__ import annotations

from dataclasses import dataclass, field

# ───────────────────────────────────────────────────────────
# Risk levels ordered for comparison
# ───────────────────────────────────────────────────────────
_LEVEL_ORDER = {"low": 0, "medium": 1, "high": 2, "critical": 3}


def _max_level(a: str, b: str) -> str:
    """Return the higher risk level."""
    return a if _LEVEL_ORDER.get(a, 0) >= _LEVEL_ORDER.get(b, 0) else b


# ───────────────────────────────────────────────────────────
# Intent class → default risk / approval
# Mirrors _INTENT_RULES in intent_normalizer.py
# ───────────────────────────────────────────────────────────
_INTENT_DEFAULTS: dict[str, tuple[str, bool]] = {
    "build":        ("medium", False),
    "deploy":       ("high",   True),
    "operate":      ("high",   True),
    "files":        ("high",   True),
    "mesh_ops":     ("medium", True),
    "self_buff":    ("high",   True),
    "chat":         ("low",    False),
    "automate":     ("medium", True),
    "automation":   ("medium", True),
    "strategy":     ("medium", False),
    "research":     ("low",    False),
    "audit":        ("low",    False),
    "media":        ("low",    False),
    "status_check": ("low",    False),
    "general":      ("low",    False),
}

# ───────────────────────────────────────────────────────────
# Keyword escalators — if any of these appear in the raw_text,
# risk is escalated to at least the specified level.
# Format: (keyword_or_phrase, escalate_to_level, reason)
# ───────────────────────────────────────────────────────────
_KEYWORD_ESCALATORS: list[tuple[str, str, str]] = [
    # Critical actions
    ("production",      "high",     "Targets production environment"),
    ("prod",            "high",     "Targets production environment (shorthand)"),
    ("delete",          "high",     "Destructive operation: delete"),
    ("drop table",      "critical", "Destructive operation: drop table"),
    ("drop database",   "critical", "Destructive operation: drop database"),
    ("rm -rf",          "critical", "Destructive operation: recursive force delete"),
    ("format disk",     "critical", "Destructive operation: format disk"),
    ("wipe",            "critical", "Destructive operation: wipe"),
    ("destroy",         "critical", "Destructive operation: destroy"),
    ("kill",            "high",     "Destructive operation: kill process"),
    ("sudo",            "high",     "Elevated privileges requested"),
    ("root",            "high",     "Root-level access mentioned"),
    # Financial / external
    ("billing",         "high",     "Financial operation: billing"),
    ("payment",         "high",     "Financial operation: payment"),
    ("api key",         "high",     "Secret management: API key"),
    ("token",           "medium",   "Secret management: token reference"),
    ("secret",          "high",     "Secret management: secret reference"),
    ("credential",      "high",     "Secret management: credentials"),
    ("password",        "high",     "Secret management: password"),
    # Network / external
    ("external api",    "medium",   "External API interaction"),
    ("third party",     "medium",   "Third-party service interaction"),
    ("webhook",         "medium",   "Webhook configuration"),
    # State mutations
    ("migration",       "high",     "Database migration"),
    ("rollback",        "high",     "Rollback operation"),
    ("revert",          "medium",   "Revert operation"),
]

# ───────────────────────────────────────────────────────────
# Surface trust modifiers — CLI operations are higher-trust
# than Discord because the operator has physical access.
# ───────────────────────────────────────────────────────────
_SURFACE_ESCALATORS: dict[str, tuple[str, str]] = {
    # surface → (minimum_risk, reason)
    "discord": ("low",    "Discord surface — standard trust level"),
    "cli":     ("low",    "CLI surface — operator has physical access"),
    "api":     ("medium", "API surface — verify caller identity"),
    "web":     ("medium", "Web surface — verify caller identity"),
}


@dataclass
class RiskAssessment:
    """Output of the risk scorer."""
    risk_level: str
    requires_approval: bool
    escalation_reasons: list[str] = field(default_factory=list)

    def to_dict(self) -> dict:
        return {
            "risk_level": self.risk_level,
            "requires_approval": self.requires_approval,
            "escalation_reasons": self.escalation_reasons,
        }


class RiskScorer:
    """
    Rule-based risk scorer. No external dependencies.

    Usage:
        scorer = RiskScorer()
        assessment = scorer.score(
            intent_class="deploy",
            raw_text="deploy the status page to production",
            source_surface="discord"
        )
        print(assessment.risk_level)        # "high"
        print(assessment.requires_approval) # True
    """

    def score(
        self,
        intent_class: str,
        raw_text: str,
        source_surface: str = "cli",
    ) -> RiskAssessment:
        """
        Compute risk assessment for a task.

        Args:
            intent_class: Classified intent (from IntentNormalizer)
            raw_text: Original user input
            source_surface: Where the request originated ("discord"|"cli"|"api"|"web")

        Returns:
            RiskAssessment with risk_level, requires_approval, and escalation_reasons.
        """
        reasons: list[str] = []

        # 1. Start with intent class default
        default_risk, default_approval = _INTENT_DEFAULTS.get(
            intent_class, ("low", False)
        )
        current_level = default_risk
        needs_approval = default_approval

        # 2. Apply keyword escalators
        lowered = raw_text.lower() if raw_text else ""
        for keyword, escalate_to, reason in _KEYWORD_ESCALATORS:
            if keyword.lower() in lowered:
                if _LEVEL_ORDER.get(escalate_to, 0) > _LEVEL_ORDER.get(current_level, 0):
                    reasons.append(f"↑ {reason}")
                    current_level = _max_level(current_level, escalate_to)

        # 3. Apply surface trust modifier
        if source_surface in _SURFACE_ESCALATORS:
            surface_min, surface_reason = _SURFACE_ESCALATORS[source_surface]
            if _LEVEL_ORDER.get(surface_min, 0) > _LEVEL_ORDER.get(current_level, 0):
                reasons.append(f"↑ {surface_reason}")
                current_level = _max_level(current_level, surface_min)

        # 4. Force approval for high/critical regardless of intent default
        if _LEVEL_ORDER.get(current_level, 0) >= _LEVEL_ORDER.get("high", 0):
            needs_approval = True

        return RiskAssessment(
            risk_level=current_level,
            requires_approval=needs_approval,
            escalation_reasons=reasons,
        )
