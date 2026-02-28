#!/usr/bin/env python3
"""Select a Heiwa deploy identity from conversation intent text."""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]
IDENTITY_MAP = ROOT / "config/identities/profiles.json"


def normalize_text(value: str) -> str:
    return re.sub(r"\s+", " ", value.lower()).strip()


def keyword_score(text: str, keywords: list[str]) -> tuple[int, list[str]]:
    score = 0
    hits: list[str] = []
    for key in keywords:
        key_norm = normalize_text(key)
        if key_norm and key_norm in text:
            score += len(key_norm)
            hits.append(key)
    return score, hits


def load_profiles() -> dict:
    return json.loads(IDENTITY_MAP.read_text(encoding="utf-8"))


def select_identity(text: str, profiles: dict) -> dict:
    identities = profiles.get("identities", [])
    default_id = profiles.get("default_identity", "")

    best = None
    best_score = -1
    for identity in identities:
        score, hits = keyword_score(text, identity.get("trigger_keywords", []))
        candidate = dict(identity)
        candidate["match_score"] = score
        candidate["matched_keywords"] = hits
        if score > best_score:
            best = candidate
            best_score = score

    if best and best_score > 0:
        return {
            "selection_reason": "keyword_match",
            "selected": best,
            "fallback_used": False,
        }

    fallback = next((i for i in identities if i.get("id") == default_id), None)
    if not fallback and identities:
        fallback = identities[0]

    fallback_copy = dict(fallback) if fallback else {}
    fallback_copy["match_score"] = 0
    fallback_copy["matched_keywords"] = []

    return {
        "selection_reason": "fallback_default",
        "selected": fallback_copy,
        "fallback_used": True,
    }


def main() -> int:
    parser = argparse.ArgumentParser(description="Select Heiwa identity from conversation text")
    parser.add_argument("--text", required=True, help="Conversation text or task summary")
    parser.add_argument("--json", action="store_true", help="Print machine-readable JSON only")
    args = parser.parse_args()

    profiles = load_profiles()
    text = normalize_text(args.text)
    result = select_identity(text, profiles)

    payload = {
        "input": args.text,
        "identity_map": str(IDENTITY_MAP),
        "selection": result,
    }

    if args.json:
        print(json.dumps(payload, indent=2))
        return 0

    selected = (result.get("selected") or {})
    print("--- HEIWA IDENTITY SELECTOR ---")
    print(f"Identity: {selected.get('id', 'unknown')}")
    print(f"Reason: {result.get('selection_reason')}")
    print(f"Matched keywords: {', '.join(selected.get('matched_keywords', [])) or '(none)'}")

    targets = selected.get("targets", {})
    actions = selected.get("actions", {})
    print(f"Target tool/runtime: {targets.get('tool', 'n/a')} / {targets.get('runtime', 'n/a')}")
    print(f"Dispatch lanes: {', '.join(targets.get('dispatch', [])) or '(none)'}")

    if actions:
        print(f"CI workflow: {actions.get('ci_workflow') or '(none)'}")
        print(f"Railway service: {actions.get('railway_service') or '(none)'}")
        print(f"Discord channel: {actions.get('discord_channel') or '(none)'}")

    print("\nJSON payload:")
    print(json.dumps(payload, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())