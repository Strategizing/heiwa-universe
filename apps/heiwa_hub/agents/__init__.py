# fleets/hub/agents/__init__.py
"""Heiwa Cloud Agents Package."""
from heiwa_hub.agents.base import BaseAgent, ProposalAgent

__all__ = ["BaseAgent", "ProposalAgent"]


def __getattr__(name: str):
    """Lazy imports for agents with heavy dependencies (discord, heiwa_ui)."""
    if name == "MessengerAgent":
        from heiwa_hub.agents.messenger import MessengerAgent
        return MessengerAgent
    if name == "HeartbeatAgent":
        from heiwa_hub.agents.test import HeartbeatAgent
        return HeartbeatAgent
    raise AttributeError(f"module {__name__!r} has no attribute {name!r}")
