from __future__ import annotations

import asyncio
import json
import logging
from dataclasses import replace
from typing import Any

from heiwa_hub.agents.base import BaseAgent
from heiwa_hub.cognition.compute_router import ComputeRouter
from heiwa_hub.cognition.intent_normalizer import IntentNormalizer
from heiwa_hub.cognition.planner import LocalTaskPlanner
from heiwa_hub.cognition.risk_scorer import RiskScorer
from heiwa_protocol.protocol import Subject

logger = logging.getLogger("Broker")

BROKER_ENVELOPE_VERSION = "2026-03-06"


class BrokerAgent(BaseAgent):
    """Pure NATS enrichment agent for intent, risk, routing, and step planning."""

    def __init__(self) -> None:
        super().__init__(name="heiwa-broker")
        self.planner = LocalTaskPlanner()
        self.normalizer = self.planner.normalizer
        self.risk_scorer = RiskScorer()
        self.compute_router = ComputeRouter()

    async def run(self) -> None:
        await self.connect()

        if self.nc:
            await self.nc.subscribe(Subject.BROKER_ROUTE.value, cb=self._handle_route_message)
            logger.info("📮 Broker listening on %s", Subject.BROKER_ROUTE.value)

        logger.info("🛰️ Broker active.")
        while self.running:
            await asyncio.sleep(1)

    async def _handle_route_message(self, msg) -> None:
        try:
            request = json.loads(msg.data.decode())
            response = self._enrich_request(request)
        except Exception as exc:
            logger.error("❌ Broker failed to enrich request: %s", exc)
            fallback_request = {}
            try:
                fallback_request = json.loads(msg.data.decode())
            except Exception:
                pass
            response = self._error_response(
                request=fallback_request,
                code="broker_enrichment_failed",
                message=str(exc),
            )

        if msg.reply:
            await msg.respond(json.dumps(response).encode())

    def _enrich_request(self, request: dict[str, Any]) -> dict[str, Any]:
        request_id = str(request.get("request_id", "")).strip()
        task_id = str(request.get("task_id", "")).strip()
        raw_text = str(request.get("raw_text", "") or "")
        sender_id = str(request.get("sender_id", "unknown"))
        source_surface = str(request.get("source_surface", "cli") or "cli")
        response_channel_id = request.get("response_channel_id", "cli")
        response_thread_id = request.get("response_thread_id")
        envelope_version = str(request.get("envelope_version", "")).strip()

        if not request.get("auth_validated"):
            return self._error_response(
                request=request,
                code="broker_auth_not_validated",
                message="Broker requires auth_validated=True from Spine.",
            )

        if envelope_version != BROKER_ENVELOPE_VERSION:
            return self._error_response(
                request=request,
                code="broker_envelope_version_mismatch",
                message=f"Expected envelope_version={BROKER_ENVELOPE_VERSION}.",
            )

        if not request_id or not task_id or not raw_text:
            return self._error_response(
                request=request,
                code="broker_invalid_request",
                message="request_id, task_id, and raw_text are required.",
            )

        normalized = self.normalizer.normalize(raw_text)
        risk = self.risk_scorer.score(
            intent_class=normalized.intent_class,
            raw_text=raw_text,
            source_surface=source_surface,
        )
        routed_profile = replace(
            normalized,
            risk_level=risk.risk_level,
            requires_approval=risk.requires_approval,
        )
        plan = self.planner.plan(
            task_id=task_id,
            raw_text=raw_text,
            requested_by=sender_id,
            source_channel_id=source_surface,
            source_message_id=task_id,
            response_channel_id=response_channel_id,
            response_thread_id=response_thread_id,
            intent_profile=routed_profile,
        )
        plan_payload = plan.to_dict()
        plan_payload["normalization"] = {
            **plan_payload.get("normalization", {}),
            "risk_assessment": risk.to_dict(),
        }

        route = self.compute_router.route(
            intent_class=plan.intent_class,
            risk_level=plan.risk_level,
            raw_text=raw_text,
            normalization=plan_payload.get("normalization"),
        )

        return {
            **plan_payload,
            "request_id": request_id,
            "task_id": task_id,
            "intent_class": plan.intent_class,
            "risk_level": plan.risk_level,
            "compute_class": route.compute_class,
            "assigned_worker": route.assigned_worker,
            "requires_approval": plan.requires_approval,
            "requested_by": sender_id,
            "raw_text": raw_text,
            "response_channel_id": response_channel_id,
            "response_thread_id": response_thread_id,
            "source_surface": source_surface,
            "envelope_version": envelope_version,
        }

    @staticmethod
    def _error_response(request: dict[str, Any], code: str, message: str) -> dict[str, Any]:
        return {
            "request_id": str(request.get("request_id", "")),
            "task_id": str(request.get("task_id", "")),
            "envelope_version": str(request.get("envelope_version", "")),
            "error": code,
            "message": message,
        }
