from __future__ import annotations

from dataclasses import dataclass
import json
from pathlib import Path

from heiwa_protocol.routing import normalize_privacy_level


@dataclass(slots=True)
class ComputeRoute:
    compute_class: int
    assigned_worker: str
    target_tool: str
    target_model: str
    target_runtime: str
    target_tier: str
    privacy_level: str
    rationale: str


class ComputeRouter:
    """
    Heiwa routing core.

    Converts the normalized intent/risk/privacy profile into a control-plane
    route that the broker, spine, executor, and MCP surface can all share.
    """

    def __init__(self, router_path: Path | None = None) -> None:
        root = Path(__file__).resolve().parents[3]
        self.router_path = router_path or root / "config" / "swarm" / "ai_router.json"
        self.router_config = self._load_router()
        self.registry = self.router_config.get("models", {}).get("registry", {})

    def _load_router(self) -> dict:
        try:
            return json.loads(self.router_path.read_text(encoding="utf-8"))
        except Exception:
            return {}

    def _model_for_worker(self, worker: str) -> str:
        return str(self.registry.get(worker, {}).get("id") or "")

    def _local_worker_for_intent(self, intent_class: str) -> str:
        if intent_class == "build":
            return "node_a_codegen"
        return "node_a_orchestrator"

    def _route_from_worker(
        self,
        *,
        compute_class: int,
        worker: str,
        runtime: str,
        tier: str,
        privacy_level: str,
        rationale: str,
        direct_tool: str | None = None,
    ) -> ComputeRoute:
        model_id = self._model_for_worker(worker)
        target_tool = direct_tool or "heiwa_claw"
        return ComputeRoute(
            compute_class=compute_class,
            assigned_worker=worker,
            target_tool=target_tool,
            target_model=model_id,
            target_runtime=runtime,
            target_tier=tier,
            privacy_level=privacy_level,
            rationale=rationale,
        )

    def route(
        self,
        intent_class: str,
        risk_level: str,
        raw_text: str = "",
        privacy_level: str | None = None,
    ) -> ComputeRoute:
        privacy = normalize_privacy_level(privacy_level, raw_text)
        intent = str(intent_class or "general").strip().lower() or "general"
        risk = str(risk_level or "low").strip().lower() or "low"

        # Sovereign clamp: everything remains local and off-cloud.
        if privacy == "sovereign":
            worker = self._local_worker_for_intent(intent)
            if intent in {"audit", "status_check", "chat", "files"}:
                compute_class = 1
                direct_tool = "heiwa_ops" if intent != "chat" else "heiwa_claw"
                tier = "tier1_local"
            else:
                compute_class = 2
                direct_tool = "heiwa_ops" if intent in {"deploy", "operate", "automate", "automation", "mesh_ops"} else None
                tier = "tier5_heavy_code" if intent in {"build", "self_buff"} else "tier3_orchestrator"
            return self._route_from_worker(
                compute_class=compute_class,
                worker=worker,
                runtime="macbook",
                tier=tier,
                privacy_level=privacy,
                rationale="Sovereign clamp keeps execution on local trusted infrastructure.",
                direct_tool=direct_tool,
            )

        if intent in {"audit", "status_check"}:
            return self._route_from_worker(
                compute_class=1,
                worker="node_a_orchestrator",
                runtime="both" if intent == "audit" else "railway",
                tier="tier1_local",
                privacy_level=privacy,
                rationale="Deterministic audit/status work stays on the fast local ops path.",
                direct_tool="heiwa_ops",
            )

        if intent == "files":
            return self._route_from_worker(
                compute_class=1,
                worker="node_a_codegen",
                runtime="macbook",
                tier="tier1_local",
                privacy_level=privacy,
                rationale="Filesystem mutations stay local and deterministic.",
                direct_tool="heiwa_ops",
            )

        if intent in {"deploy", "operate", "automate", "automation"}:
            return ComputeRoute(
                compute_class=4,
                assigned_worker="railway_control_plane",
                target_tool="heiwa_ops",
                target_model="",
                target_runtime="railway",
                target_tier="tier3_orchestrator",
                privacy_level=privacy,
                rationale="Operational control-plane work stays on Railway via bounded deterministic tools.",
            )

        if intent == "build":
            if risk in {"high", "critical"}:
                return self._route_from_worker(
                    compute_class=3,
                    worker="class_3_build",
                    runtime="both",
                    tier="tier5_heavy_code",
                    privacy_level=privacy,
                    rationale="High-risk build work escalates to premium HeiwaClaw routing.",
                )
            return self._route_from_worker(
                compute_class=2,
                worker="node_a_codegen",
                runtime="macbook",
                tier="tier5_heavy_code",
                privacy_level=privacy,
                rationale="Cheapest acceptable build route stays local on the codegen node.",
            )

        if intent == "media":
            return self._route_from_worker(
                compute_class=2,
                worker="node_b_media",
                runtime="both",
                tier="tier2_fast_context",
                privacy_level=privacy,
                rationale="Media work uses the localized fast-path media worker behind HeiwaClaw.",
            )

        if intent == "research":
            return self._route_from_worker(
                compute_class=3,
                worker="class_3_research",
                runtime="both",
                tier="tier6_premium_context",
                privacy_level=privacy,
                rationale="Research work routes through premium long-context infrastructure.",
            )

        if intent == "strategy":
            return self._route_from_worker(
                compute_class=3,
                worker="class_3_strategy",
                runtime="both",
                tier="tier7_supreme_court",
                privacy_level=privacy,
                rationale="Strategy work routes through adversarial-review capable premium infrastructure.",
            )

        if intent in {"mesh_ops", "self_buff"}:
            return self._route_from_worker(
                compute_class=2,
                worker="node_a_orchestrator",
                runtime="macbook",
                tier="tier3_orchestrator",
                privacy_level=privacy,
                rationale="Core Heiwa maintenance stays local-first for speed and operator control.",
                direct_tool="heiwa_ops" if intent == "mesh_ops" else None,
            )

        if intent == "chat":
            return self._route_from_worker(
                compute_class=1,
                worker="node_a_orchestrator",
                runtime="railway",
                tier="tier1_local",
                privacy_level=privacy,
                rationale="Low-latency chat uses the smallest acceptable HeiwaClaw route.",
            )

        return self._route_from_worker(
            compute_class=2,
            worker="node_a_orchestrator",
            runtime="both",
            tier="tier3_orchestrator",
            privacy_level=privacy,
            rationale="Default route keeps work on the local-first HeiwaClaw path.",
        )
