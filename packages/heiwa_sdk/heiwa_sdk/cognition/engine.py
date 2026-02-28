"""
Heiwa Cognition Engine — The Unified Brain v3.1.

Implements the Atomic Broadcast pattern: Memory -> Conscience -> Action.
Integrates LLMProvider for high-speed async streaming.
Loads "Soul" and "Identity" dynamically to ensure opinionated responses.
Judges all proposals via ConfidenceGate.
"""
import asyncio
import json
import logging
import os
import time
from datetime import datetime, timezone
from typing import Any, AsyncGenerator, Dict, List, Optional

from .provider import LLMProvider
from heiwa_sdk.db import Database, Thought
from .reasoning.confidence import ConfidenceGate, GateResult
from heiwa_identity.soul import get_soul, get_identity_meta

logger = logging.getLogger("SDK.Cognition.Engine")

class CognitionEngine:
    """
    The Unified Brain of Heiwa.
    Handles streaming, reasoning, and identity-aware generation.
    """

    def __init__(self, db: Optional[Database] = None):
        self.provider = LLMProvider()
        self.gate = ConfidenceGate(db)
        self.db = db
        
        # Identity / Soul
        self.soul = get_soul()
        self.identity = get_identity_meta()
        
        # Load root identity if available (SOTA v3.1)
        root_identity_path = os.path.join(os.getcwd(), "IDENTITY.md")
        if os.path.exists(root_identity_path):
            with open(root_identity_path, "r") as f:
                self.identity = f.read()
        
        root_soul_path = os.path.join(os.getcwd(), "SOUL.md")
        if os.path.exists(root_soul_path):
            with open(root_soul_path, "r") as f:
                self.soul = f.read()

    async def generate_stream(self, 
                               prompt: str, 
                               model: str = "google/gemini-2.0-flash", 
                               system: Optional[str] = None) -> AsyncGenerator[str, None]:
        """
        Stream tokens with identity-aware system prompts.
        """
        # Inject Soul and Identity into system prompt
        full_system = f"{self.identity}\n\n{self.soul}\n\n"
        if system:
            full_system += f"--- CONTEXTUAL INSTRUCTION ---\n{system}"
        
        async for chunk in self.provider.generate_stream(prompt, model, full_system):
            yield chunk

    async def generate(self, 
                       prompt: str, 
                       model: str = "google/gemini-2.0-flash", 
                       system: Optional[str] = None) -> str:
        """Non-streaming generation."""
        full_result = ""
        async for chunk in self.generate_stream(prompt, model, system):
            full_result += chunk
        return full_result

    async def evaluate_and_stream(self,
                                  origin: str,
                                  intent: str,
                                  instruction: str,
                                  model: str = "google/gemini-2.0-flash",
                                  system: Optional[str] = None) -> AsyncGenerator[str, None]:
        """
        The Atomic Broadcast Generation:
        1. Reason about the intent (Internal Thought).
        2. Evaluate the plan (ConfidenceGate).
        3. Stream the execution.
        """
        # 0. Context Gathering
        context = []
        if self.db:
            context = self.db.get_stream_context(limit=5)
        
        context_str = "\n".join([f"- {c['origin']}: {c['reasoning']}" for c in context])

        # 1. Internal Reasoning
        reasoning_prompt = f"Previous context:\n{context_str}\n\nReason about this task: {intent}\nInstruction: {instruction}\nProvide a 2-sentence calculated plan."
        reasoning = await self.generate(reasoning_prompt, model, system)
        
        # 2. Conscience Check
        thought = Thought(
            origin=origin,
            intent=intent,
            thought_type="proposal",
            confidence=0.85, # Default high for intent processing
            reasoning=reasoning,
            artifact={"type": "action", "instruction": instruction}
        )
        
        # Atomic Broadcast: Memory
        if self.db:
            self.db.insert_thought(thought)

        gate_result = self.gate.evaluate(thought)
        
        # Update thought with gate decision (Metadata)
        thought.metadata["gate_decision"] = gate_result.decision
        
        # Emit reasoning as a thought
        yield f"[REASONING]: {reasoning}\n[GATE]: {gate_result.decision} - {gate_result.reason}\n\n"
        
        if gate_result.decision in ["REJECT", "HOLD"]:
            yield f"⚠️ Execution blocked by Conscience: {gate_result.reason}"
            return

        # 3. Execution Stream
        async for chunk in self.provider.generate_stream(instruction, model, system):
            yield chunk

    async def close(self):
        await self.provider.close()

# Alias for backward compatibility
Cognition = CognitionEngine
