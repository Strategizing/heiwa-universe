# libs/heiwa_sdk/cognition/__init__.py
"""
Cognition Package — The Heiwa Thinking Layer.

This package contains:
- engine.py: Cognition class (Atomic Broadcast pattern)
- reasoning/: ConfidenceGate, CompetitiveReasoning, models
"""
from heiwa_sdk.cognition.engine import Cognition

__all__ = ["Cognition"]