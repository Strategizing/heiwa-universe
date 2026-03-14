from __future__ import annotations

from dataclasses import dataclass
import re


WELCOME_SUGGESTIONS: tuple[str, ...] = (
    "what should I work on next in Heiwa",
    "research how SpacetimeDB subscriptions work",
    "fix the SQLite fallback in db.py and verify it",
    "review the Railway deployment and report blockers",
    "summarize Heiwa status and propose the next highest-leverage fix",
    "deploy the hub and report the result",
)


@dataclass(slots=True)
class FastPathTurn:
    response: str
    intent: str = "conversation"
    lane: str = "scale_zero"
    tool: str = "operator_surface"
    rationale: str = "Handled locally without model dispatch."


def operator_display_name(node_name: str | None = None) -> str:
    raw = str(node_name or "").strip()
    if not raw:
        return "Devon"
    base = raw.split("@", 1)[0].strip()
    if base.lower() in {"devon", "dmcgregsauce", "macbook"}:
        return "Devon"
    cleaned = re.sub(r"[-_]+", " ", base)
    return cleaned.title() or "Devon"


def maybe_fast_path_turn(text: str, node_name: str | None = None) -> FastPathTurn | None:
    normalized = re.sub(r"\s+", " ", str(text or "").strip().lower())
    if not normalized:
        return None

    operator_name = operator_display_name(node_name)
    greeting_tokens = {
        "hi",
        "hi heiwa",
        "hello",
        "hello heiwa",
        "hey",
        "hey heiwa",
        "yo",
        "sup",
        "good morning",
        "good afternoon",
        "good evening",
    }
    if normalized in greeting_tokens:
        return FastPathTurn(
            response=f"Hey {operator_name}! What can I help you with?",
            rationale="Greeting turn resolved in the operator surface without routing or provider spend.",
        )

    if normalized in {"thanks", "thank you", "thx", "ty"}:
        return FastPathTurn(
            response="Anytime. Want me to research, build, review, deploy, or audit something?",
            rationale="Low-complexity acknowledgement resolved locally.",
        )

    if normalized in {"help", "what can you do", "what should i ask", "suggestions"}:
        suggestions = "\n".join(f"- {item}" for item in WELCOME_SUGGESTIONS[:4])
        return FastPathTurn(
            response=(
                "I can route research, implementation, review, ops, and deploy work. Try one of these:\n\n"
                f"{suggestions}"
            ),
            rationale="Help prompt resolved locally with curated operator suggestions.",
        )

    return None
