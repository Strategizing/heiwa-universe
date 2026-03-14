from __future__ import annotations

import asyncio
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))
sys.path.insert(0, str(ROOT / "packages/heiwa_protocol"))
sys.path.insert(0, str(ROOT / "packages/heiwa_identity"))
sys.path.insert(0, str(ROOT / "apps"))

from heiwa_protocol.routing import BrokerRouteResult
from heiwa_sdk.heiwaclaw import HeiwaClawGateway


class DummyToolMesh:
    def __init__(self, response: tuple[int, str]) -> None:
        self.response = response

    async def execute(self, tool: str, instruction: str, model: str | None = None, extra_env: dict[str, str] | None = None) -> tuple[int, str]:
        return self.response


def _route(**overrides: object) -> BrokerRouteResult:
    payload = {
        "request_id": "req-test",
        "task_id": "task-test",
        "raw_text": "reply with exactly OK",
        "source_surface": "cli",
        "intent_class": "chat",
        "risk_level": "low",
        "privacy_level": "local",
        "compute_class": 1,
        "assigned_worker": "node_a_orchestrator",
        "target_tool": "heiwa_claw",
        "target_model": "ollama/llama-4-scout:q4_k_m",
        "target_runtime": "railway",
        "target_tier": "tier1_local",
        "requires_approval": False,
        "rationale": "test route",
    }
    payload.update(overrides)
    return BrokerRouteResult.from_payload(payload)


def main() -> int:
    failures: list[str] = []

    gateway = HeiwaClawGateway(ROOT)
    gateway.tool_mesh = DummyToolMesh((2, "❌ [HEIWA TOOLMESH] Tool 'heiwa_reflex' is unavailable."))
    gateway._execute_via_runtime_engine = lambda route, dispatch, instruction: "OK"  # type: ignore[method-assign]

    code, output = asyncio.run(gateway.execute(_route(), "reply with exactly OK"))
    if code != 0 or output != "OK":
        failures.append(f"runtime fallback should satisfy Railway chat lane, got code={code} output={output!r}")

    gateway = HeiwaClawGateway(ROOT)
    gateway.tool_mesh = DummyToolMesh((2, "❌ [HEIWA TOOLMESH] Tool 'heiwa_code' is unavailable."))
    gateway._execute_via_runtime_engine = lambda route, dispatch, instruction: "SHOULD NOT RUN"  # type: ignore[method-assign]

    code, output = asyncio.run(
        gateway.execute(
            _route(
                intent_class="build",
                risk_level="high",
                compute_class=3,
                assigned_worker="class_3_build",
                target_model="openai-codex/gpt-5.3-codex",
                target_tier="tier5_heavy_code",
            ),
            "implement the change",
        )
    )
    if code == 0 or "unavailable" not in output.lower():
        failures.append("build lane should not silently degrade to runtime-text fallback")

    if failures:
        print("HeiwaClaw runtime fallback test FAILED")
        for failure in failures:
            print(f" - {failure}")
        return 1

    print("HeiwaClaw runtime fallback test PASSED")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
