from __future__ import annotations

from dataclasses import asdict, dataclass
import json
from pathlib import Path
from typing import Any

from heiwa_identity.selector import load_profiles, select_identity


def _display_name_from_id(identity_id: str) -> str:
    return " ".join(part.capitalize() for part in str(identity_id or "").replace("_", "-").split("-") if part)


@dataclass(slots=True)
class HeiwaCell:
    cell_id: str
    display_name: str
    identity_id: str
    description: str
    gateway_tool: str
    target_runtime: str
    required_capabilities: list[str]
    trigger_keywords: list[str]
    roster: list[dict[str, Any]]
    models: dict[str, list[str]]
    report_channel: str | None = None
    work_channel: str | None = None
    maturity: str = "seed"
    install_source: str = "config/identities/profiles.json"

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


class HeiwaCellCatalog:
    """Materializes installable HeiwaCells from the identity manifest."""

    def __init__(self, root_dir: Path | None = None) -> None:
        self.root = root_dir or Path(__file__).resolve().parents[3]
        self._profiles = load_profiles()

    @property
    def profiles(self) -> dict[str, Any]:
        return self._profiles

    def list_cells(self) -> list[HeiwaCell]:
        cells: list[HeiwaCell] = []
        for identity in self._profiles.get("identities", []):
            targets = dict(identity.get("targets") or {})
            actions = dict(identity.get("actions") or {})
            cells.append(
                HeiwaCell(
                    cell_id=str(identity.get("id") or ""),
                    display_name=_display_name_from_id(identity.get("id") or ""),
                    identity_id=str(identity.get("id") or ""),
                    description=str(identity.get("description") or ""),
                    gateway_tool=str(targets.get("tool") or "openclaw"),
                    target_runtime=str(targets.get("runtime") or "macbook@heiwa-node-a"),
                    required_capabilities=list(targets.get("required_capabilities") or []),
                    trigger_keywords=list(identity.get("trigger_keywords") or []),
                    roster=list(identity.get("cells") or []),
                    models=dict(identity.get("models") or {}),
                    report_channel=actions.get("report_channel"),
                    work_channel=actions.get("discord_channel"),
                )
            )
        return cells

    def get_cell(self, cell_id: str) -> HeiwaCell | None:
        needle = str(cell_id or "").strip()
        for cell in self.list_cells():
            if cell.cell_id == needle:
                return cell
        return None

    def recommend(self, prompt: str) -> dict[str, Any]:
        selection = select_identity(prompt, self._profiles)
        selected = dict(selection.get("selected") or {})
        identity_id = str(selected.get("id") or "")
        cell = self.get_cell(identity_id)
        return {
            "selection_reason": selection.get("selection_reason", "fallback_default"),
            "fallback_used": bool(selection.get("fallback_used")),
            "matched_keywords": list(selected.get("matched_keywords") or []),
            "match_score": int(selected.get("match_score") or 0),
            "cell": cell.to_dict() if cell else None,
        }

    def to_public_dict(self) -> dict[str, Any]:
        return {
            "default_identity": self._profiles.get("default_identity", ""),
            "cells": [cell.to_dict() for cell in self.list_cells()],
        }

    def to_json(self) -> str:
        return json.dumps(self.to_public_dict(), indent=2)
