from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Any

_BASE_CLASS_BY_INTENT: dict[str, int] = {
    "audit": 1,
    "chat": 1,
    "files": 1,
    "general": 1,
    "status_check": 1,
    "build": 2,
    "media": 2,
    "mesh_ops": 2,
    "self_buff": 2,
    "research": 3,
    "strategy": 3,
    "automate": 4,
    "automation": 4,
    "deploy": 4,
    "operate": 4,
}

_CLASS2_PREFERENCES: dict[str, tuple[str, ...]] = {
    "build": ("node_a_codegen", "node_b_gpu_builder", "node_a_orchestrator"),
    "media": ("node_b_media", "node_b_gpu_builder", "node_a_codegen"),
    "mesh_ops": ("node_a_orchestrator", "node_a_codegen", "node_b_gpu_builder"),
    "self_buff": ("node_a_codegen", "node_b_gpu_builder", "node_a_orchestrator"),
}

_CLASS3_PREFERENCES: dict[str, tuple[str, ...]] = {
    "build": ("class_3_build", "class_3_low_cost_remote", "class_3_fast_remote"),
    "research": ("class_3_research", "class_3_low_cost_remote", "class_3_fast_remote"),
    "strategy": ("class_3_strategy", "class_3_research", "class_3_low_cost_remote"),
}

_SOVEREIGN_KEYWORDS = (
    "sovereign",
    "private data",
    "local only",
    "local-only",
    "stay local",
    "on device",
    "on-device",
)


@dataclass
class ComputeRoute:
    compute_class: int
    assigned_worker: str
    privacy_level: str
    rationale: str

    def to_dict(self) -> dict[str, Any]:
        return {
            "compute_class": self.compute_class,
            "assigned_worker": self.assigned_worker,
            "privacy_level": self.privacy_level,
            "rationale": self.rationale,
        }


class ComputeRouter:
    """Resolve a broker compute class and worker hint from intent/risk policy."""

    def __init__(self, router_path: Path | None = None) -> None:
        root = Path(__file__).resolve().parents[3]
        self.router_path = router_path or root / "config" / "swarm" / "ai_router.json"
        self.config = self._load_config()
        self.registry = self.config.get("models", {}).get("registry", {})

    def route(
        self,
        *,
        intent_class: str,
        risk_level: str,
        raw_text: str = "",
        privacy_level: str | None = None,
        normalization: dict[str, Any] | None = None,
    ) -> ComputeRoute:
        resolved_privacy = self._resolve_privacy_level(
            privacy_level=privacy_level,
            normalization=normalization,
            raw_text=raw_text,
        )

        compute_class = _BASE_CLASS_BY_INTENT.get(intent_class, 1)

        if intent_class == "build" and risk_level in {"high", "critical"}:
            compute_class = 3
        elif intent_class in {"chat", "general"} and risk_level in {"high", "critical"}:
            compute_class = 2

        if resolved_privacy == "sovereign" and compute_class > 2:
            compute_class = 2

        assigned_worker = self._assigned_worker_for(intent_class=intent_class, compute_class=compute_class)
        rationale = (
            f"intent={intent_class} risk={risk_level} privacy={resolved_privacy} "
            f"-> class={compute_class} worker={assigned_worker}"
        )

        return ComputeRoute(
            compute_class=compute_class,
            assigned_worker=assigned_worker,
            privacy_level=resolved_privacy,
            rationale=rationale,
        )

    def _load_config(self) -> dict[str, Any]:
        if not self.router_path.exists():
            return {}
        return json.loads(self.router_path.read_text())

    def _resolve_privacy_level(
        self,
        *,
        privacy_level: str | None,
        normalization: dict[str, Any] | None,
        raw_text: str,
    ) -> str:
        direct = str(privacy_level or "").strip().lower()
        if direct:
            return direct

        normalized_privacy = str((normalization or {}).get("privacy_level", "")).strip().lower()
        if normalized_privacy:
            return normalized_privacy

        lowered = (raw_text or "").lower()
        if any(keyword in lowered for keyword in _SOVEREIGN_KEYWORDS):
            return "sovereign"

        return "standard"

    def _assigned_worker_for(self, *, intent_class: str, compute_class: int) -> str:
        if compute_class == 1:
            return self._host_for("node_a_orchestrator", fallback="macbook@heiwa-node-a")

        if compute_class == 2:
            preferences = _CLASS2_PREFERENCES.get(
                intent_class,
                ("node_a_codegen", "node_a_orchestrator", "node_b_gpu_builder"),
            )
            for key in preferences:
                host = self._host_for(key)
                if host:
                    return host
            return "macbook@heiwa-node-a"

        if compute_class == 3:
            preferences = _CLASS3_PREFERENCES.get(
                intent_class,
                ("class_3_low_cost_remote", "class_3_fast_remote", "class_3_research"),
            )
            for key in preferences:
                host = self._host_for(key)
                if host:
                    return host
            return "cloud@premium-remote"

        return "railway"

    def _host_for(self, registry_key: str, fallback: str = "") -> str:
        entry = self.registry.get(registry_key, {})
        host_node = str(entry.get("host_node", "")).strip()
        return host_node or fallback
