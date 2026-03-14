"""
Smoke test: rate-group-aware routing and cascade.

Verifies:
1. RateGroupLedger tracks usage per group
2. ComputeRouter cascades to next provider when group is exhausted
3. Throttle detection triggers cooldown
"""

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
for pkg in ["heiwa_sdk", "heiwa_protocol", "heiwa_identity"]:
    path = str(ROOT / f"packages/{pkg}")
    if path not in sys.path:
        sys.path.insert(0, path)
if str(ROOT / "apps") not in sys.path:
    sys.path.insert(0, str(ROOT / "apps"))


def test_ledger_basic():
    from heiwa_sdk.rate_ledger import RateGroupLedger
    ledger = RateGroupLedger()

    # Local groups are unlimited
    assert ledger.has_capacity("local_ollama")
    assert ledger.remaining("local_ollama") is None

    # Claude code has limited capacity
    assert ledger.has_capacity("claude_code")
    remaining = ledger.remaining("claude_code")
    assert remaining is not None and remaining > 0

    # Record usage and verify count decreases
    ledger.record("claude_code")
    new_remaining = ledger.remaining("claude_code")
    assert new_remaining == remaining - 1


def test_ledger_exhaustion():
    from heiwa_sdk.rate_ledger import RateGroupLedger
    ledger = RateGroupLedger()

    # Exhaust a small group
    for _ in range(15):
        ledger.record("siliconflow")

    assert not ledger.has_capacity("siliconflow")
    assert ledger.remaining("siliconflow") == 0


def test_ledger_throttle():
    from heiwa_sdk.rate_ledger import RateGroupLedger
    ledger = RateGroupLedger()

    assert ledger.has_capacity("cerebras")
    ledger.record_throttle("cerebras")
    assert not ledger.has_capacity("cerebras")


def test_cascade_on_exhaustion():
    from heiwa_hub.cognition.compute_router import ComputeRouter
    from heiwa_sdk.rate_ledger import RateGroupLedger, _ledger_lock
    import heiwa_sdk.rate_ledger as rate_mod

    # Install a fresh ledger for this test
    ledger = RateGroupLedger(ROOT / "config" / "swarm" / "ai_router.json")
    with _ledger_lock:
        rate_mod._ledger = ledger

    router = ComputeRouter(ROOT / "config" / "swarm" / "ai_router.json")

    # Research routes to class_3_research (google_gemini_cli)
    route = router.route("research", "low")
    assert route.compute_class == 3
    original_worker = route.assigned_worker

    # Exhaust the google_gemini_cli group
    for _ in range(55):
        ledger.record("google_gemini_cli")

    # Now research should cascade inside the research/strategy/review family,
    # not jump immediately to a generic fast-reasoner lane.
    cascaded = router.route("research", "low")
    assert cascaded.compute_class == 3
    assert cascaded.assigned_worker == "class_3_strategy"
    assert cascaded.target_model == "google-antigravity/gemini-3-flash"
    assert cascaded.rationale.startswith("Rate cascade")

    # Cleanup: reset the singleton
    with _ledger_lock:
        rate_mod._ledger = None


def test_build_cascade_prefers_review_family():
    from heiwa_hub.cognition.compute_router import ComputeRouter
    from heiwa_sdk.rate_ledger import RateGroupLedger, _ledger_lock
    import heiwa_sdk.rate_ledger as rate_mod

    ledger = RateGroupLedger(ROOT / "config" / "swarm" / "ai_router.json")
    with _ledger_lock:
        rate_mod._ledger = ledger

    router = ComputeRouter(ROOT / "config" / "swarm" / "ai_router.json")

    route = router.route("build", "high")
    assert route.assigned_worker == "class_3_build"

    for _ in range(30):
        ledger.record("openai_codex")

    cascaded = router.route("build", "high")
    assert cascaded.compute_class == 3
    assert cascaded.assigned_worker == "class_3_review"
    assert cascaded.target_model == "claude-code/claude-opus-4-6"
    assert cascaded.rationale.startswith("Rate cascade")

    # Cleanup: reset the singleton
    with _ledger_lock:
        rate_mod._ledger = None


def test_status_report():
    from heiwa_sdk.rate_ledger import RateGroupLedger
    ledger = RateGroupLedger()

    status = ledger.status()
    assert "claude_code" in status
    assert "local_ollama" in status
    assert status["local_ollama"]["unlimited"] is True
    assert status["claude_code"]["available"] is True
    assert "used" in status["claude_code"]
    assert "max" in status["claude_code"]


if __name__ == "__main__":
    tests = [
        ("ledger_basic", test_ledger_basic),
        ("ledger_exhaustion", test_ledger_exhaustion),
        ("ledger_throttle", test_ledger_throttle),
        ("cascade_on_exhaustion", test_cascade_on_exhaustion),
        ("build_cascade_prefers_review_family", test_build_cascade_prefers_review_family),
        ("status_report", test_status_report),
    ]
    passed = 0
    for name, fn in tests:
        try:
            fn()
            print(f"  PASS  {name}")
            passed += 1
        except Exception as e:
            print(f"  FAIL  {name}: {e}")

    print(f"\n{passed}/{len(tests)} passed.")
    sys.exit(0 if passed == len(tests) else 1)
