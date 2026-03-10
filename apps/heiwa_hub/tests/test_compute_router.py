"""
Phase B Gate 3 - Compute Router Verification

Validates deterministic intent/risk routing against the March 6, 2026
execution-tier policy and enforces the sovereign privacy clamp.
"""
from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))
sys.path.insert(0, str(ROOT / "packages/heiwa_protocol"))
sys.path.insert(0, str(ROOT / "packages/heiwa_identity"))
sys.path.insert(0, str(ROOT / "apps"))

from heiwa_hub.cognition.compute_router import ComputeRouter


def main() -> int:
    router = ComputeRouter()
    passed = 0
    failed = 0
    failures: list[dict[str, object]] = []

    test_cases = [
        {"name": "audit stays cpu-first", "intent": "audit", "risk": "low", "expect_class": 1},
        {"name": "files stay local cpu-first", "intent": "files", "risk": "high", "expect_class": 1},
        {"name": "build defaults to local class 2", "intent": "build", "risk": "medium", "expect_class": 2},
        {"name": "high-risk build escalates to class 3", "intent": "build", "risk": "high", "expect_class": 3},
        {"name": "media uses local gpu class 2", "intent": "media", "risk": "low", "expect_class": 2},
        {"name": "research uses premium remote class 3", "intent": "research", "risk": "low", "expect_class": 3},
        {"name": "strategy uses premium remote class 3", "intent": "strategy", "risk": "medium", "expect_class": 3},
        {"name": "deploy uses cloud persistence class 4", "intent": "deploy", "risk": "high", "expect_class": 4},
        {"name": "operate uses cloud persistence class 4", "intent": "operate", "risk": "high", "expect_class": 4},
        {"name": "automate uses cloud persistence class 4", "intent": "automate", "risk": "medium", "expect_class": 4},
        {
            "name": "sovereign research clamps to local",
            "intent": "research",
            "risk": "low",
            "raw_text": "research this but keep sovereign data local only",
            "expect_class": 2,
        },
        {
            "name": "sovereign deploy clamps to local",
            "intent": "deploy",
            "risk": "high",
            "privacy_level": "sovereign",
            "expect_class": 2,
        },
    ]

    for case in test_cases:
        route = router.route(
            intent_class=str(case["intent"]),
            risk_level=str(case["risk"]),
            raw_text=str(case.get("raw_text", "")),
            privacy_level=str(case.get("privacy_level", "")) or None,
        )
        expect_class = int(case["expect_class"])
        class_ok = route.compute_class == expect_class
        worker_ok = bool(route.assigned_worker)
        sovereign_ok = route.compute_class <= 2 if route.privacy_level == "sovereign" else True

        if class_ok and worker_ok and sovereign_ok:
            passed += 1
        else:
            failed += 1
            failures.append(
                {
                    "name": case["name"],
                    "expected_class": expect_class,
                    "actual_class": route.compute_class,
                    "assigned_worker": route.assigned_worker,
                    "privacy_level": route.privacy_level,
                    "rationale": route.rationale,
                }
            )

    total = passed + failed
    print(f"\n{'=' * 60}")
    print("  Compute Router Verification")
    print(f"{'=' * 60}")
    print(f"  Total cases:    {total}")
    print(f"  Passed:         {passed}")
    print(f"  Failed:         {failed}")
    print(f"  Result:         {'PASS' if failed == 0 else 'FAIL'}")
    print(f"{'=' * 60}")

    if failures:
        print("\n  Failures:")
        print(f"  {'-' * 56}")
        for failure in failures:
            print(f"  {failure['name']}")
            print(
                f"      Class:    expected={failure['expected_class']} actual={failure['actual_class']}"
            )
            print(f"      Worker:   {failure['assigned_worker']}")
            print(f"      Privacy:  {failure['privacy_level']}")
            print(f"      Why:      {failure['rationale']}")
            print()

    return 0 if failed == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
