from __future__ import annotations

from heiwa_hub.cognition.compute_router import ComputeRouter
from heiwa_hub.cognition.intent_normalizer import IntentNormalizer
from heiwa_hub.cognition.risk_scorer import RiskScorer
from heiwa_protocol.routing import BrokerRouteRequest, BrokerRouteResult


class BrokerEnrichmentService:
    """Typed broker enrichment pipeline shared by broker, spine, and MCP."""

    def __init__(self) -> None:
        self.normalizer = IntentNormalizer()
        self.scorer = RiskScorer()
        self.router = ComputeRouter()

    def enrich(self, request: BrokerRouteRequest) -> BrokerRouteResult:
        profile = self.normalizer.normalize(request.raw_text)
        assessment = self.scorer.score(
            intent_class=profile.intent_class,
            raw_text=request.raw_text,
            source_surface=request.source_surface,
        )
        route = self.router.route(
            intent_class=profile.intent_class,
            risk_level=assessment.risk_level,
            raw_text=request.raw_text,
            privacy_level=request.privacy_level,
        )

        normalization = profile.to_dict()
        normalization["preferred_runtime"] = route.target_runtime
        normalization["preferred_tool"] = route.target_tool
        normalization["preferred_tier"] = route.target_tier
        normalization["risk_level"] = assessment.risk_level

        return BrokerRouteResult(
            request_id=request.request_id,
            task_id=request.task_id,
            envelope_version=request.envelope_version,
            raw_text=request.raw_text,
            source_surface=request.source_surface,
            intent_class=profile.intent_class,
            risk_level=assessment.risk_level,
            privacy_level=route.privacy_level,
            compute_class=route.compute_class,
            assigned_worker=route.assigned_worker,
            target_tool=route.target_tool,
            target_model=route.target_model,
            target_runtime=route.target_runtime,
            target_tier=route.target_tier,
            requires_approval=assessment.requires_approval or profile.requires_approval,
            rationale=route.rationale,
            confidence=profile.confidence,
            escalation_reasons=assessment.escalation_reasons,
            assumptions=profile.assumptions,
            missing_details=profile.missing_details,
            normalization=normalization,
        )
