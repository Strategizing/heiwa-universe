from __future__ import annotations

import time
from typing import Any

from .db import Database


class HubStateService:
    """Shared state access for HTTP/MCP/websocket surfaces."""

    def __init__(self, db: Database | None = None) -> None:
        self.db = db or Database()

    def get_public_status(self, minutes: int = 60) -> dict[str, Any]:
        summary = self.db.get_model_usage_summary(minutes=minutes)
        live_nodes = self.db.list_nodes(status="ONLINE")
        return {
            "status": "OPERATIONAL",
            "state_backend": self.db.state_backend,
            "active_models": len(summary),
            "live_nodes": len(live_nodes),
            "usage_summary": summary,
            "timestamp": time.time(),
        }

    def get_recent_runs(self, limit: int = 10) -> list[dict[str, Any]]:
        return self.db.get_runs(limit=max(1, int(limit)))
