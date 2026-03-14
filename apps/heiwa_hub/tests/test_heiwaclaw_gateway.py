from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))
sys.path.insert(0, str(ROOT / "packages/heiwa_protocol"))
sys.path.insert(0, str(ROOT / "packages/heiwa_identity"))
sys.path.insert(0, str(ROOT / "apps"))

from heiwa_hub.cognition.enrichment import BrokerEnrichmentService
from heiwa_protocol.routing import BrokerRouteRequest
from heiwa_sdk.heiwaclaw import HeiwaClawGateway


def main() -> int:
    enrichment = BrokerEnrichmentService()
    gateway = HeiwaClawGateway(ROOT)

    cases = [
        (
            "premium build routes through HeiwaClaw",
            BrokerRouteRequest(
                request_id="req-build",
                task_id="task-build",
                raw_text="implement a build pipeline refactor with sudo support",
                source_surface="cli",
                auth_validated=True,
            ),
            {"compute_class": 3, "gateway_tool": "heiwa_claw", "adapter_tool": "heiwa_code"},
        ),
        (
            "research resolves to premium websocket path",
            BrokerRouteRequest(
                request_id="req-research",
                task_id="task-research",
                raw_text="research the latest SpacetimeDB Python SDK changes",
                source_surface="cli",
                auth_validated=True,
            ),
            {"compute_class": 3, "gateway_tool": "heiwa_claw", "adapter_tool": "heiwa_gemini"},
        ),
        (
            "strategy resolves to antigravity oauth lane",
            BrokerRouteRequest(
                request_id="req-strategy",
                task_id="task-strategy",
                raw_text="architect the control plane and adversarially review the roadmap",
                source_surface="cli",
                auth_validated=True,
            ),
            {"compute_class": 3, "gateway_tool": "heiwa_claw", "adapter_tool": "heiwa_claw"},
        ),
        (
            "audit stays on deterministic direct path",
            BrokerRouteRequest(
                request_id="req-audit",
                task_id="task-audit",
                raw_text="audit the active config and verify local health",
                source_surface="cli",
                auth_validated=True,
            ),
            {"compute_class": 1, "gateway_tool": "heiwa_ops", "adapter_tool": "heiwa_ops"},
        ),
    ]

    failures: list[str] = []
    for name, request, expect in cases:
        result = enrichment.enrich(request)
        dispatch = gateway.resolve(result)
        if result.compute_class != expect["compute_class"]:
            failures.append(f"{name}: expected compute_class={expect['compute_class']} actual={result.compute_class}")
        if dispatch.gateway_tool != expect["gateway_tool"]:
            failures.append(f"{name}: expected gateway_tool={expect['gateway_tool']} actual={dispatch.gateway_tool}")
        if dispatch.adapter_tool != expect["adapter_tool"]:
            failures.append(f"{name}: expected adapter_tool={expect['adapter_tool']} actual={dispatch.adapter_tool}")

    if failures:
        print("HeiwaClaw gateway contract test FAILED")
        for failure in failures:
            print(f" - {failure}")
        return 1

    print("HeiwaClaw gateway contract test PASSED")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
