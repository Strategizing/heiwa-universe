import asyncio
import json
import logging
import time
from typing import Any, Dict

from apps.heiwa_hub.agents.base import BaseAgent
from apps.heiwa_hub.cognition.intent_normalizer import IntentNormalizer
from apps.heiwa_hub.cognition.risk_scorer import RiskScorer
from apps.heiwa_hub.cognition.compute_router import ComputeRouter
from heiwa_protocol.protocol import Subject

logger = logging.getLogger("BrokerAgent")

class BrokerAgent(BaseAgent):
    """
    BrokerAgent — NATS request-reply enrichment agent.
    Enriches task envelopes with intent classification, risk scoring, and compute routing.
    """
    def __init__(self):
        super().__init__(name="BrokerAgent")
        self.normalizer = IntentNormalizer()
        self.scorer = RiskScorer()
        self.router = ComputeRouter()

    async def run(self):
        """Main execution loop: Connect to NATS and subscribe to the broker route."""
        await self.connect()
        
        if self.nc:
            # Subscribe to the broker route subject for request-reply enrichment
            await self.nc.subscribe(Subject.BROKER_ROUTE.value, cb=self._handle_broker_request)
            logger.info(f"👂 [{self.name}] Listening for enrichment requests on {Subject.BROKER_ROUTE.value}")

        # Keep alive
        while self.running:
            await asyncio.sleep(1)

    async def _handle_broker_request(self, msg):
        """
        Processes a BrokerRouteRequest and replies with a BrokerRouteResult.
        """
        start_time = time.time()
        try:
            # 1. Parse Request
            payload = json.loads(msg.data.decode())
            # Extract data from standard payload or root
            request_data = payload.get("data", payload)
            
            request_id = request_data.get("request_id")
            task_id = request_data.get("task_id")
            raw_text = request_data.get("raw_text", "")
            source_surface = request_data.get("source_surface", "cli")
            envelope_version = request_data.get("envelope_version", "2026-03-06")

            logger.info(f"🔍 [{self.name}] Enriching task {task_id} (req: {request_id})")

            # 2. Intent Classification
            profile = self.normalizer.normalize(raw_text)
            intent_class = profile.intent_class

            # 3. Risk Scoring
            assessment = self.scorer.score(
                intent_class=intent_class,
                raw_text=raw_text,
                source_surface=source_surface
            )

            # 4. Compute Routing
            routing = self.router.route(
                intent_class=intent_class,
                risk_level=assessment.risk_level
            )

            # 5. Construct Result
            result = {
                "request_id": request_id,
                "task_id": task_id,
                "intent_class": intent_class,
                "risk_level": assessment.risk_level,
                "compute_class": routing.compute_class,
                "assigned_worker": routing.assigned_worker,
                "requires_approval": assessment.requires_approval or profile.requires_approval,
                "steps": [], # Future extraction: Planner will fill this
                "normalization": profile.to_dict(),
                "raw_text": raw_text,
                "response_channel_id": request_data.get("response_channel_id"),
                "response_thread_id": request_data.get("response_thread_id"),
                "envelope_version": envelope_version
            }

            # 6. Reply via NATS
            if msg.reply:
                await self.nc.publish(msg.reply, json.dumps(result).encode())
                duration = round((time.time() - start_time) * 1000, 2)
                logger.info(f"✅ [{self.name}] Replied to {request_id} in {duration}ms (Intent: {intent_class}, Risk: {assessment.risk_level})")
            else:
                logger.warning(f"⚠️ [{self.name}] Received request {request_id} but no reply subject provided.")

        except Exception as e:
            logger.error(f"❌ [{self.name}] Error enriching request: {e}")
            # If we fail, we don't reply, letting Spine timeout and fallback

if __name__ == "__main__":
    agent = BrokerAgent()
    try:
        asyncio.run(agent.run())
    except KeyboardInterrupt:
        asyncio.run(agent.shutdown())
