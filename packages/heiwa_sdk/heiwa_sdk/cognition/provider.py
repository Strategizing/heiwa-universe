"""
Compatibility shim — routes through HeiwaClawGateway instead of
maintaining a parallel provider implementation.

Legacy callers that imported LLMProvider or TokenUsage from here
will still work, but execution now flows through the real gateway
path: ProviderRegistry -> HeiwaClawGateway -> ToolMesh.
"""

import logging
from dataclasses import dataclass
from typing import Optional, AsyncGenerator

from heiwa_sdk.heiwaclaw import HeiwaClawGateway

logger = logging.getLogger("SDK.Cognition.Provider")


@dataclass
class TokenUsage:
    prompt_tokens: int = 0
    completion_tokens: int = 0
    total_tokens: int = 0


class LLMProvider:
    """Thin shim over HeiwaClawGateway for backward compatibility."""

    def __init__(self):
        self.gateway = HeiwaClawGateway()

    async def generate_stream(
        self,
        prompt: str,
        model: str,
        system: Optional[str] = None,
    ) -> AsyncGenerator[str, None]:
        """
        Non-streaming execution through the gateway.  Yields the full
        result as a single chunk.  Streaming will be added when the
        adapter wrappers support it.
        """
        from heiwa_protocol.routing import BrokerRouteResult

        route = BrokerRouteResult.from_payload({
            "request_id": "compat",
            "task_id": "compat",
            "raw_text": prompt,
            "intent_class": "general",
            "risk_level": "low",
            "privacy_level": "local",
            "compute_class": 2,
            "assigned_worker": "local",
            "target_tool": "heiwa_reflex",
            "target_model": model,
            "target_runtime": "macbook",
            "target_tier": "tier2_local",
            "requires_approval": False,
        })

        dispatch = self.gateway.resolve(route)
        instruction = f"{system}\n\n{prompt}" if system else prompt
        exit_code, output = await self.gateway.execute(dispatch, instruction)

        if exit_code == 0:
            yield output
        else:
            logger.error("Gateway execution failed (exit %d): %s", exit_code, output)
            yield f"Execution failed: {output}"

    async def close(self):
        pass
