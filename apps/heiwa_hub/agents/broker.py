"""
Broker enrichment — no longer a standalone agent.

Spine calls BrokerEnrichmentService.enrich() directly. This module
exists for backward compatibility and re-exports the service.
"""

from heiwa_hub.cognition.enrichment import BrokerEnrichmentService

__all__ = ["BrokerEnrichmentService"]
