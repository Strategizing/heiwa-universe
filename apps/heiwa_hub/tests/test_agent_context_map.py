from __future__ import annotations

from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]


def main() -> int:
    required_files = [
        ROOT / "HEIWA.md",
        ROOT / "SOUL.md",
        ROOT / "rooms" / "control-plane.md",
        ROOT / "rooms" / "execution.md",
        ROOT / "rooms" / "orchestration.md",
        ROOT / "rooms" / "infra.md",
        ROOT / "rooms" / "sdk.md",
    ]
    failures: list[str] = []

    for path in required_files:
        if not path.exists():
            failures.append(f"missing required context file: {path}")

    if not failures:
        heiwa_text = (ROOT / "HEIWA.md").read_text(encoding="utf-8")
        if "Task Routing Table" not in heiwa_text:
            failures.append("HEIWA.md is missing the task routing table")
        if "rooms/control-plane.md" not in heiwa_text:
            failures.append("HEIWA.md is missing room index references")

        soul_text = (ROOT / "SOUL.md").read_text(encoding="utf-8")
        if "compatibility shim" not in soul_text.lower():
            failures.append("SOUL.md should explain that it is a compatibility shim")
        if "HEIWA.md" not in soul_text:
            failures.append("SOUL.md should redirect to HEIWA.md")

    if failures:
        print("Agent context map test FAILED")
        for failure in failures:
            print(f" - {failure}")
        return 1

    print("Agent context map test PASSED")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
