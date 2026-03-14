from __future__ import annotations

import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))
sys.path.insert(0, str(ROOT / "packages/heiwa_protocol"))
sys.path.insert(0, str(ROOT / "packages/heiwa_identity"))
sys.path.insert(0, str(ROOT / "apps"))

from heiwa_sdk.cells import HeiwaCellCatalog


def main() -> int:
    catalog = HeiwaCellCatalog(ROOT)
    public = catalog.to_public_dict()
    cells = public.get("cells", [])
    failures: list[str] = []

    if len(cells) < 4:
        failures.append(f"expected at least 4 cells from profiles, got {len(cells)}")

    ids = {cell.get("cell_id") for cell in cells}
    for required in {"operator-general", "codex-builder", "research-scout", "architect-strategist"}:
        if required not in ids:
            failures.append(f"missing cell {required}")

    recommendation = catalog.recommend("implement this code patch and prep the PR")
    if (recommendation.get("cell") or {}).get("cell_id") != "codex-builder":
        failures.append(f"unexpected recommendation: {recommendation}")

    if failures:
        print("HeiwaCells catalog test FAILED")
        for failure in failures:
            print(f" - {failure}")
        return 1

    print("HeiwaCells catalog test PASSED")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
