"""
ConfidenceGate: The Superego of the Swarm.
Determines if a Thought executes immediately or waits for the Human Key.
"""
import logging
from dataclasses import dataclass
from ..db import Database, Thought

logger = logging.getLogger("confidence")


@dataclass
class GateResult:
    """Result of a ConfidenceGate evaluation."""
    decision: str  # EXECUTE, QUEUE, HOLD, REJECT
    adjusted_score: float
    reason: str


class ConfidenceGate:
    """
    The superego of the Swarm.
    Determines if a Thought acts immediately or waits for the Human Key.
    
    Thresholds (The Velvet Rope):
      - AUTO_EXECUTE (0.90): High confidence, verified. Go.
      - SOFT_QUEUE (0.70): Good idea, but requires human eyes.
      - HARD_HOLD (0.60): Risky. Halting execution chain.
      - REJECT (0.40): Hallucination or unsafe. Discard.
    """

    # Thresholds (The Velvet Rope)
    AUTO_EXECUTE = 0.90
    SOFT_QUEUE = 0.70  # Queue but keep thinking
    HARD_HOLD = 0.60   # Stop everything, human needed
    REJECT = 0.40      # Garbage thought, discard

    def __init__(self, db: Database = None):
        self.db = db

    def evaluate(self, thought: Thought, context: dict = None) -> GateResult:
        """
        Evaluate a Thought and return a gate decision.
        
        Args:
            thought: The Thought object to evaluate
            context: Optional context dict with keys like:
                     - critic_approved: bool
                     - critic_rejected: bool
        
        Returns:
            GateResult with decision, adjusted_score, and reason
        """
        base = thought.confidence
        context = context or {}

        # 1. The Critic's Tax (DeepSeek-R1 Validation)
        # If the Critic reviewed this and said "LGTM", we boost.
        if context.get("critic_approved"):
            base += 0.15
        elif context.get("critic_rejected"):
            base -= 0.30

        # 2. Risk Profiling (The "Don't Touch Prod" Tax)
        artifact_type = thought.artifact.get("type", "none") if thought.artifact else "none"

        # Code/Config changes are inherently risky
        if artifact_type in ["code", "patch", "config"]:
            base -= 0.10

        # Browser actions are medium risk (could click wrong button)
        if thought.intent and "browser" in thought.intent.lower():
            base -= 0.05

        # 3. Success History (The Reputation Score)
        # TODO: Implement granular Agent Reputation in DB
        # if self.db:
        #     history = self.db.get_agent_reputation(thought.origin)
        #     base *= history.trust_multiplier

        # Clamp score to [0.0, 1.0]
        base = max(0.0, min(1.0, base))

        # 4. The Verdict
        if base >= self.AUTO_EXECUTE:
            return GateResult(
                decision="EXECUTE",
                adjusted_score=base,
                reason="High confidence, verified."
            )
        elif base >= self.SOFT_QUEUE:
            return GateResult(
                decision="QUEUE",
                adjusted_score=base,
                reason="Good idea, but requires human eyes."
            )
        elif base >= self.HARD_HOLD:
            return GateResult(
                decision="HOLD",
                adjusted_score=base,
                reason="Risky. Halting execution chain."
            )
        else:
            return GateResult(
                decision="REJECT",
                adjusted_score=base,
                reason="Hallucination or unsafe."
            )