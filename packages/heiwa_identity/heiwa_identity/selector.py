import json
import re
import os
from pathlib import Path
from typing import Dict, List, Tuple, Optional

# Constants
ROOT = Path(__file__).resolve().parents[3]
IDENTITY_MAP = ROOT / "config/identities/profiles.json"

def normalize_text(value: str) -> str:
    return re.sub(r"\s+", " ", value.lower()).strip()

def keyword_score(text: str, keywords: List[str]) -> Tuple[int, List[str]]:
    score = 0
    hits: List[str] = []
    for key in keywords:
        key_norm = normalize_text(key)
        if key_norm and key_norm in text:
            score += len(key_norm)
            hits.append(key)
    return score, hits

def load_profiles() -> dict:
    if not IDENTITY_MAP.exists():
        return {"identities": [], "default_identity": ""}
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
