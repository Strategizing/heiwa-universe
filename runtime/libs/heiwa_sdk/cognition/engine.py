"""
Cognition: The interface between the Blackboard (DB) and the Nervous System (NATS).
Handles the atomic broadcast: Memory -> Conscience -> Action.
"""
import json
import logging
from datetime import datetime, timezone

from .db import Database, Thought
from .nervous_system import HeiwaNervousSystem
from .reasoning.confidence import ConfidenceGate, GateResult

logger = logging.getLogger("cognition")


class Cognition:
    """
    The interface between the Blackboard (DB) and the Nervous System (NATS).
    
    Implements the Atomic Broadcast pattern:
    1. Write to Blackboard (Memory)
    2. Judge with ConfidenceGate (Conscience)
    3. Publish to NATS (Action)
    """

    # Skill mapping for Antigravity integration
    SKILL_MAP = {
        "browser": "skill.browser.navigate",
        "code": "skill.editor.patch",
        "terminal": "skill.terminal.run",
        "file": "skill.file.write",
    }

    def __init__(self, db: Database = None, nerve: HeiwaNervousSystem = None):
        self.db = db
        self.nerve = nerve
        self.gate = ConfidenceGate(db)

    async def broadcast_thought(self, thought: Thought) -> GateResult:
        """
        The Atomic Broadcast:
        1. Write to Blackboard (Memory)
        2. Judge with ConfidenceGate (Conscience)
        3. Publish to NATS (Action)
        
        Returns:
            GateResult if thought was a proposal, None otherwise
        """

        # 1. Memorize
        if self.db and not self.db.insert_thought(thought):
            logger.error("Failed to write thought to Blackboard. Aborting.")
            return None

        # 2. Judge
        # We only judge 'proposals' (actions). 'Observations' pass freely.
        gate_result = None
        if thought.thought_type == "proposal":
            gate_result = self.gate.evaluate(thought)

            # Update thought metadata with gate decision
            thought.metadata["gate_decision"] = gate_result.decision
            thought.metadata["gate_score"] = gate_result.adjusted_score
            logger.info(
                f"Gate Decision: {gate_result.decision} "
                f"(score: {gate_result.adjusted_score:.2f}) - {gate_result.reason}"
            )

        # 3. Publish (The Signal)
        if self.nerve:
            # Subject format: heiwa.thought.{type}.{origin}
            subject = f"heiwa.thought.{thought.thought_type}.{thought.origin}"

            payload = thought.to_dict()
            if gate_result:
                payload["gate"] = {
                    "decision": gate_result.decision,
                    "score": gate_result.adjusted_score,
                    "reason": gate_result.reason,
                }

            await self.nerve.publish_directive(subject, payload)
            logger.info(f"Published thought to: {subject}")

            # 4. Direct Action (If EXECUTE)
            if gate_result and gate_result.decision == "EXECUTE":
                await self._trigger_execution(thought)

        return gate_result

    async def _trigger_execution(self, thought: Thought):
        """
        Translates a Thought into a NATS Directive for the Antigravity Node.
        Maps artifact types to Antigravity Skills.
        """
        if not self.nerve:
            logger.warning("No nerve connection, cannot trigger execution")
            return

        # Deduce target skill from artifact type or intent
        target_subject = "heiwa.directives.generic"
        artifact_type = thought.artifact.get("type", "none") if thought.artifact else "none"

        # Map artifact type to skill
        if artifact_type == "code" or artifact_type == "patch":
            target_subject = self.SKILL_MAP.get("code", target_subject)
        elif artifact_type == "config":
            target_subject = self.SKILL_MAP.get("file", target_subject)
        elif artifact_type == "ref":
            # Reference artifacts often need browser verification
            target_subject = self.SKILL_MAP.get("browser", target_subject)

        # Check intent for browser keywords
        if thought.intent:
            intent_lower = thought.intent.lower()
            if any(kw in intent_lower for kw in ["browser", "navigate", "url", "localhost", "verify layout"]):
                target_subject = self.SKILL_MAP.get("browser", target_subject)
            elif any(kw in intent_lower for kw in ["terminal", "run", "execute", "command"]):
                target_subject = self.SKILL_MAP.get("terminal", target_subject)

        # Build directive payload
        directive_payload = {
            "task_id": thought.stream_id,
            "intent": thought.intent,
            "artifact": thought.artifact,
            "origin": thought.origin,
            "timestamp": datetime.now(timezone.utc).isoformat(),
        }

        # Attach short-term memory context if DB available
        if self.db:
            directive_payload["context"] = self.db.get_stream_context(limit=5)

        # "Muscle, wake up."
        await self.nerve.publish_directive(target_subject, directive_payload)
        logger.info(f"Triggered Execution: {target_subject} for {thought.stream_id}")

    async def create_and_broadcast(
        self,
        origin: str,
        intent: str,
        thought_type: str,
        confidence: float,
        reasoning: str = None,
        artifact: dict = None,
        parent_id: str = None,
        tags: list = None,
    ) -> tuple[Thought, GateResult]:
        """
        Convenience method to create and broadcast a thought in one call.
        
        Returns:
            Tuple of (Thought, GateResult)
        """
        thought = Thought(
            origin=origin,
            intent=intent,
            thought_type=thought_type,
            confidence=confidence,
            reasoning=reasoning,
            artifact=artifact,
            parent_id=parent_id,
            tags=tags,
        )
        gate_result = await self.broadcast_thought(thought)
        return thought, gate_result
