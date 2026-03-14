from __future__ import annotations

from dataclasses import dataclass, field
import json
from pathlib import Path
from typing import Any


_LOCAL_PROVIDERS = {"ollama", "local", "vllm", "litellm"}


@dataclass(slots=True)
class ProviderConfig:
    name: str
    adapter_tool: str
    gateway_tool: str = "heiwa_claw"
    transport: str = "gateway_websocket"
    auth_kind: str = "unknown"
    rate_group: str = ""
    cli_command: str = ""
    direct_execution: bool = False
    websocket_preferred: bool = True
    env: dict[str, str] = field(default_factory=dict)


class ProviderRegistry:
    """Loads provider execution metadata from the Heiwa router config."""

    def __init__(self, root_dir: Path | None = None) -> None:
        self.root = root_dir or Path(__file__).resolve().parents[3]
        self.router_path = self.root / "config" / "swarm" / "ai_router.json"
        self.router_config = self._load_router()
        self.providers = dict(self.router_config.get("providers") or {})
        self.model_registry = dict((self.router_config.get("models") or {}).get("registry") or {})

    def _load_router(self) -> dict[str, Any]:
        try:
            return json.loads(self.router_path.read_text(encoding="utf-8"))
        except Exception:
            return {}

    def provider_for_worker(self, worker: str) -> str:
        match = dict(self.model_registry.get(worker) or {})
        return str(match.get("provider") or "")

    @staticmethod
    def provider_for_model(model_id: str) -> str:
        value = str(model_id or "").strip()
        return value.split("/", 1)[0] if "/" in value else value

    def resolve(self, provider_name: str) -> ProviderConfig:
        payload = dict(self.providers.get(provider_name) or {})
        if not payload:
            return self._default_provider(provider_name)

        adapter_tool = str(payload.get("adapter_tool") or "").strip() or self._default_provider(provider_name).adapter_tool
        gateway_tool = str(payload.get("gateway_tool") or "heiwa_claw").strip() or "heiwa_claw"
        transport = str(payload.get("transport") or "gateway_websocket").strip() or "gateway_websocket"
        auth_kind = str(payload.get("auth_kind") or "unknown").strip() or "unknown"
        rate_group = str(payload.get("rate_group") or provider_name).strip() or provider_name
        cli_command = str(payload.get("cli_command") or "").strip()
        direct_execution = bool(payload.get("direct_execution"))
        websocket_preferred = bool(payload.get("websocket_preferred", transport == "gateway_websocket"))
        env_payload = dict(payload.get("env") or {})
        env = {str(key): str(value) for key, value in env_payload.items()}

        return ProviderConfig(
            name=provider_name,
            adapter_tool=adapter_tool,
            gateway_tool=gateway_tool,
            transport=transport,
            auth_kind=auth_kind,
            rate_group=rate_group,
            cli_command=cli_command,
            direct_execution=direct_execution,
            websocket_preferred=websocket_preferred,
            env=env,
        )

    def _default_provider(self, provider_name: str) -> ProviderConfig:
        provider = str(provider_name or "").strip() or "unknown"
        if provider in _LOCAL_PROVIDERS:
            return ProviderConfig(
                name=provider,
                adapter_tool="heiwa_reflex",
                gateway_tool="heiwa_claw",
                transport="local_http",
                auth_kind="local_runtime",
                rate_group=provider,
                websocket_preferred=False,
            )

        return ProviderConfig(
            name=provider,
            adapter_tool="heiwa_claw",
            gateway_tool="heiwa_claw",
            transport="gateway_websocket",
            auth_kind="cli_or_gateway",
            rate_group=provider,
            websocket_preferred=True,
        )
