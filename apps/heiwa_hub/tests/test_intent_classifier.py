"""
Phase B Gate 2 — Intent Classifier Accuracy Test

Runs all 50 test cases from intent_classifier_test_set.json against the
current keyword-only IntentNormalizer and reports baseline accuracy.

Acceptance criteria: ≥95% accuracy (≥48/50 correct).
"""
from __future__ import annotations

import json
import sys
from pathlib import Path

# Wire up imports
ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))
sys.path.insert(0, str(ROOT / "packages/heiwa_protocol"))
sys.path.insert(0, str(ROOT / "packages/heiwa_identity"))
sys.path.insert(0, str(ROOT / "apps"))

from heiwa_hub.cognition.intent_normalizer import IntentNormalizer

TEST_SET_PATH = Path(__file__).parent / "intent_classifier_test_set.json"
ACCURACY_THRESHOLD = 0.95


def main() -> int:
    with open(TEST_SET_PATH) as f:
        data = json.load(f)

    cases = data["cases"]
    normalizer = IntentNormalizer(engine=None)  # keyword-only, no LLM

    correct = 0
    failures: list[dict] = []

    for case in cases:
        result = normalizer.normalize(case["input"])
        actual = result.intent_class
        expected = case["expected_class"]

        if actual == expected:
            correct += 1
        else:
            failures.append({
                "id": case["id"],
                "input": case["input"][:80],
                "expected": expected,
                "actual": actual,
                "category": case["category"],
                "notes": case.get("notes", ""),
            })

    total = len(cases)
    accuracy = correct / total if total else 0.0

    print(f"\n{'='*60}")
    print(f"  Intent Classifier Baseline Accuracy Report")
    print(f"{'='*60}")
    print(f"  Total cases:    {total}")
    print(f"  Correct:        {correct}")
    print(f"  Failed:         {len(failures)}")
    print(f"  Accuracy:       {accuracy:.1%}")
    print(f"  Threshold:      {ACCURACY_THRESHOLD:.0%}")
    print(f"  Result:         {'✅ PASS' if accuracy >= ACCURACY_THRESHOLD else '❌ FAIL'}")
    print(f"{'='*60}")

    if failures:
        print(f"\n  Misclassifications:")
        print(f"  {'─'*56}")
        for f_case in failures:
            print(f"  #{f_case['id']:>2} [{f_case['category']}]")
            print(f"      Input:    {f_case['input']}...")
            print(f"      Expected: {f_case['expected']}")
            print(f"      Actual:   {f_case['actual']}")
            print(f"      Notes:    {f_case['notes'][:80]}")
            print()

    # Output machine-readable summary
    summary = {
        "total": total,
        "correct": correct,
        "accuracy": round(accuracy, 4),
        "threshold": ACCURACY_THRESHOLD,
        "passed": accuracy >= ACCURACY_THRESHOLD,
        "failures": failures,
    }
    summary_path = Path(__file__).parent / "intent_classifier_baseline.json"
    with open(summary_path, "w") as f:
        json.dump(summary, f, indent=2)
    print(f"  Summary written to: {summary_path}")

    return 0 if accuracy >= ACCURACY_THRESHOLD else 1


if __name__ == "__main__":
    raise SystemExit(main())
