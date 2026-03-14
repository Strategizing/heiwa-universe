from __future__ import annotations

from dataclasses import asdict, dataclass, field
import logging
from pathlib import Path
from typing import Any

from heiwa_protocol.routing import BrokerRouteResult

from .provider_registry import ProviderRegistry
from .tool_mesh import ToolMesh


logger = logging.getLogger("SDK.HeiwaClaw")

_RUNTIME_ENGINE_INTENTS = {"chat", "general", "research", "strategy"}
_RUNTIME_ENGINE_UNAVAILABLE_MARKERS = (
    "is unavailable",
    "command not found",
    "no such file",
    "not installed",
)


@dataclass(slots=True)
class HeiwaClawDispatch:
    gateway_tool: str
    adapter_tool: str
    provider: str
    target_model: str
    target_runtime: str
    session_id: str
    transport: str
    rate_group: str
    auth_kind: str
    websocket_preferred: bool
    direct_execution: bool
    rationale: str
    adapter_env: dict[str, str] = field(default_factory=dict)

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


class HeiwaClawGateway:
    """Unified execution gateway for all model/provider routing."""

    def __init__(self, root_dir: Path):
        self.root = root_dir
        self.tool_mesh = ToolMesh(root_dir)
        self.providers = ProviderRegistry(root_dir)

    def _provider_for(self, route: BrokerRouteResult) -> str:
        provider = self.providers.provider_for_worker(route.assigned_worker)
        if provider:
            return provider
        provider = self.providers.provider_for_model(route.target_model)
        return provider or "local"

    def resolve(self, payload: BrokerRouteResult | dict[str, Any]) -> HeiwaClawDispatch:
        route = payload if isinstance(payload, BrokerRouteResult) else BrokerRouteResult.from_payload(payload)
        if route.target_tool == "heiwa_ops":
            return HeiwaClawDispatch(
                gateway_tool=route.target_tool,
                adapter_tool="heiwa_ops",
                provider="local",
                target_model=route.target_model,
                target_runtime=route.target_runtime,
                session_id=f"heiwa-ops-{route.task_id}",
                transport="direct_exec",
                rate_group="deterministic_ops",
                auth_kind="none",
                websocket_preferred=False,
                direct_execution=True,
                rationale=route.rationale,
            )

        provider = self._provider_for(route)
        provider_config = self.providers.resolve(provider)

        return HeiwaClawDispatch(
            gateway_tool=provider_config.gateway_tool,
            adapter_tool=provider_config.adapter_tool,
            provider=provider,
            target_model=route.target_model,
            target_runtime=route.target_runtime,
            session_id=f"heiwa-{route.task_id}",
            transport=provider_config.transport,
            rate_group=provider_config.rate_group,
            auth_kind=provider_config.auth_kind,
            websocket_preferred=provider_config.websocket_preferred,
            direct_execution=provider_config.direct_execution,
            rationale=route.rationale,
            adapter_env=provider_config.env,
        )

    @staticmethod
    def _runtime_engine_allowed(route: BrokerRouteResult, dispatch: HeiwaClawDispatch) -> bool:
        runtime = str(dispatch.target_runtime or route.target_runtime or "").strip().lower()
        intent = str(route.intent_class or "").strip().lower()
        return runtime in {"railway", "cloud"} and intent in _RUNTIME_ENGINE_INTENTS

    @staticmethod
    def _runtime_engine_complexity(route: BrokerRouteResult) -> str:
        tier = str(route.target_tier or "").strip().lower()
        if tier in {"tier6_premium_context", "tier7_supreme_court"}:
            return "high"
        if tier in {"tier2_fast_context", "tier3_orchestrator", "tier4_pooled_orchestrator", "tier5_heavy_code"}:
            return "medium"
        return "low"

    def _execute_via_runtime_engine(
        self,
        route: BrokerRouteResult,
        dispatch: HeiwaClawDispatch,
        instruction: str,
    ) -> str:
        try:
            from heiwa_hub.cognition.llm_local import LocalLLMEngine
        except Exception as exc:
            logger.info("Runtime engine unavailable for %s: %s", dispatch.session_id, exc)
            return ""

        engine = LocalLLMEngine()
        result = engine.generate(
            prompt=instruction,
            complexity=self._runtime_engine_complexity(route),
            runtime=dispatch.target_runtime or route.target_runtime or "railway",
        )
        if result:
            logger.info(
                "Runtime engine satisfied %s via %s/%s fallback.",
                dispatch.session_id,
                route.intent_class,
                dispatch.provider,
            )
        return result.strip()

    def _should_prefer_runtime_engine(
        self,
        route: BrokerRouteResult,
        dispatch: HeiwaClawDispatch,
    ) -> bool:
        if not self._runtime_engine_allowed(route, dispatch):
            return False
        return dispatch.adapter_tool == "heiwa_reflex"

    def _should_retry_with_runtime_engine(
        self,
        route: BrokerRouteResult,
        dispatch: HeiwaClawDispatch,
        exit_code: int,
        output: str,
    ) -> bool:
        if exit_code == 0 or not self._runtime_engine_allowed(route, dispatch):
            return False
        lowered = str(output or "").lower()
        return any(marker in lowered for marker in _RUNTIME_ENGINE_UNAVAILABLE_MARKERS)

    async def execute(self, payload: BrokerRouteResult | dict[str, Any], instruction: str) -> tuple[int, str]:
        route = payload if isinstance(payload, BrokerRouteResult) else BrokerRouteResult.from_payload(payload)
        dispatch = self.resolve(route)
        env = {
            "HEIWA_GATEWAY_TRANSPORT": dispatch.transport,
            "HEIWA_PROVIDER": dispatch.provider,
            "HEIWA_RATE_GROUP": dispatch.rate_group,
            "HEIWA_AUTH_KIND": dispatch.auth_kind,
            "OPENCLAW_EXEC_MODE": "gateway" if dispatch.websocket_preferred else "local",
            "OPENCLAW_SESSION_ID": dispatch.session_id,
        }
        env.update(dispatch.adapter_env)
        if self._should_prefer_runtime_engine(route, dispatch):
            runtime_result = self._execute_via_runtime_engine(route, dispatch, instruction)
            if runtime_result:
                return 0, runtime_result

        exit_code, output = await self.tool_mesh.execute(
            dispatch.adapter_tool,
            instruction,
            model=dispatch.target_model or None,
            extra_env=env,
        )
        if self._should_retry_with_runtime_engine(route, dispatch, exit_code, output):
            runtime_result = self._execute_via_runtime_engine(route, dispatch, instruction)
            if runtime_result:
                return 0, runtime_result

        # Record usage in the rate ledger
        self._record_rate_usage(dispatch, exit_code, output)

        return exit_code, output

    @staticmethod
    def _record_rate_usage(dispatch: HeiwaClawDispatch, exit_code: int, output: str) -> None:
        """Record rate-group usage after execution. Detects throttle signals."""
        if dispatch.rate_group in {"deterministic_ops", ""} or not dispatch.rate_group:
            return
        try:
            from .rate_ledger import get_rate_ledger
            ledger = get_rate_ledger()
        except Exception:
            return

        lowered = str(output or "").lower()
        throttle_markers = ("rate limit", "429", "too many requests", "quota exceeded", "throttled")
        if exit_code != 0 and any(m in lowered for m in throttle_markers):
            ledger.record_throttle(dispatch.rate_group)
        else:
            ledger.record(dispatch.rate_group)
