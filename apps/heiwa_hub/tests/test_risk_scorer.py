"""
Phase B Gate 1 — Risk Scorer v1 Verification

Tests the rule-based risk scorer against known inputs and validates:
1. Intent class defaults produce correct base risk levels
2. Keyword escalators correctly bump risk upward
3. Surface modifiers apply correctly
4. Critical operations always require approval
"""
from __future__ import annotations

import sys
from pathlib import Path

# Wire up imports
ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))
sys.path.insert(0, str(ROOT / "packages/heiwa_protocol"))
sys.path.insert(0, str(ROOT / "packages/heiwa_identity"))
sys.path.insert(0, str(ROOT / "apps"))

from heiwa_hub.cognition.risk_scorer import RiskScorer


def main() -> int:
    scorer = RiskScorer()
    passed = 0
    failed = 0
    failures = []

    test_cases = [
        # --- Intent class defaults ---
        {
            "name": "build: default medium, no approval",
            "intent": "build", "text": "create a CLI tool", "surface": "cli",
            "expect_risk": "medium", "expect_approval": False,
        },
        {
            "name": "deploy: default high, approval required",
            "intent": "deploy", "text": "deploy to staging", "surface": "cli",
            "expect_risk": "high", "expect_approval": True,
        },
        {
            "name": "chat: default low, no approval",
            "intent": "chat", "text": "hello", "surface": "discord",
            "expect_risk": "low", "expect_approval": False,
        },
        {
            "name": "research: default low, no approval",
            "intent": "research", "text": "analyze costs", "surface": "cli",
            "expect_risk": "low", "expect_approval": False,
        },
        {
            "name": "operate: default high, approval required",
            "intent": "operate", "text": "fix the bug", "surface": "cli",
            "expect_risk": "high", "expect_approval": True,
        },
        # --- Keyword escalators ---
        {
            "name": "build + production keyword → high",
            "intent": "build", "text": "build and deploy to production", "surface": "cli",
            "expect_risk": "high", "expect_approval": True,
        },
        {
            "name": "audit + delete keyword → high",
            "intent": "audit", "text": "scan for stale files and delete them", "surface": "cli",
            "expect_risk": "high", "expect_approval": True,
        },
        {
            "name": "build + rm -rf → critical",
            "intent": "build", "text": "create a cleanup script that runs rm -rf /tmp/cache", "surface": "cli",
            "expect_risk": "critical", "expect_approval": True,
        },
        {
            "name": "research + token keyword → medium (no change, already low → medium)",
            "intent": "research", "text": "analyze the token rotation strategy", "surface": "cli",
            "expect_risk": "medium", "expect_approval": False,
        },
        {
            "name": "chat + drop table → critical",
            "intent": "chat", "text": "hey can you drop table users for me?", "surface": "discord",
            "expect_risk": "critical", "expect_approval": True,
        },
        {
            "name": "build + secret + credential → high",
            "intent": "build", "text": "build a credential rotation tool for API secrets", "surface": "cli",
            "expect_risk": "high", "expect_approval": True,
        },
        {
            "name": "automate + migration → high",
            "intent": "automate", "text": "automate the database migration nightly", "surface": "cli",
            "expect_risk": "high", "expect_approval": True,
        },
        # --- Surface modifiers ---
        {
            "name": "research + API surface → medium floor",
            "intent": "research", "text": "summarize the docs", "surface": "api",
            "expect_risk": "medium", "expect_approval": False,
        },
        {
            "name": "chat + web surface → medium floor",
            "intent": "chat", "text": "hello there", "surface": "web",
            "expect_risk": "medium", "expect_approval": False,
        },
        # --- Edge cases ---
        {
            "name": "unknown intent → low default",
            "intent": "banana", "text": "do something", "surface": "cli",
            "expect_risk": "low", "expect_approval": False,
        },
        {
            "name": "empty text → intent default only",
            "intent": "deploy", "text": "", "surface": "cli",
            "expect_risk": "high", "expect_approval": True,
        },
        {
            "name": "all 7 primary: strategy",
            "intent": "strategy", "text": "design a roadmap", "surface": "cli",
            "expect_risk": "medium", "expect_approval": False,
        },
        {
            "name": "all 7 primary: audit",
            "intent": "audit", "text": "validate the schema", "surface": "cli",
            "expect_risk": "low", "expect_approval": False,
        },
        {
            "name": "all 7 primary: media",
            "intent": "media", "text": "render an image", "surface": "cli",
            "expect_risk": "low", "expect_approval": False,
        },
        {
            "name": "all 7 primary: automate",
            "intent": "automate", "text": "set up a cron job", "surface": "cli",
            "expect_risk": "medium", "expect_approval": True,
        },
        {
            "name": "multi-keyword escalation stacks to highest",
            "intent": "build", "text": "build a production deploy script with sudo and rm -rf cleanup",
            "surface": "discord",
            "expect_risk": "critical", "expect_approval": True,
        },
    ]

    for tc in test_cases:
        result = scorer.score(
            intent_class=tc["intent"],
            raw_text=tc["text"],
            source_surface=tc["surface"],
        )
        risk_ok = result.risk_level == tc["expect_risk"]
        approval_ok = result.requires_approval == tc["expect_approval"]

        if risk_ok and approval_ok:
            passed += 1
        else:
            failed += 1
            failures.append({
                "name": tc["name"],
                "expected_risk": tc["expect_risk"],
                "actual_risk": result.risk_level,
                "expected_approval": tc["expect_approval"],
                "actual_approval": result.requires_approval,
                "reasons": result.escalation_reasons,
            })

    total = passed + failed
    print(f"\n{'='*60}")
    print(f"  Risk Scorer v1 Verification")
    print(f"{'='*60}")
    print(f"  Total cases:    {total}")
    print(f"  Passed:         {passed}")
    print(f"  Failed:         {failed}")
    print(f"  Result:         {'✅ PASS' if failed == 0 else '❌ FAIL'}")
    print(f"{'='*60}")

    if failures:
        print(f"\n  Failures:")
        print(f"  {'─'*56}")
        for f in failures:
            print(f"  {f['name']}")
            print(f"      Risk:     expected={f['expected_risk']} actual={f['actual_risk']}")
            print(f"      Approval: expected={f['expected_approval']} actual={f['actual_approval']}")
            print(f"      Reasons:  {f['reasons']}")
            print()

    return 0 if failed == 0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
