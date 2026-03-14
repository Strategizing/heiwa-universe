from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))
sys.path.insert(0, str(ROOT / "packages/heiwa_protocol"))
sys.path.insert(0, str(ROOT / "packages/heiwa_identity"))
sys.path.insert(0, str(ROOT / "apps"))

from heiwa_sdk.bench import HeiwaBench


def main() -> int:
    bench = HeiwaBench(ROOT)
    summary = bench.run()

    if not summary.get("ok"):
        print("HeiwaBench FAILED")
        for failure in summary.get("failures", []):
            print(
                " - {suite}/{case} field={field} expected={expected!r} actual={actual!r}".format(
                    **failure
                )
            )
        return 1

    if summary.get("total_cases", 0) < 3:
        print(f"HeiwaBench FAILED: expected >=3 cases, got {summary.get('total_cases')}")
        return 1

    print("HeiwaBench PASSED")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
