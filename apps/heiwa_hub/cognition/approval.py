from __future__ import annotations

import os
import threading
import time
from dataclasses import asdict, dataclass
from typing import Any


@dataclass
class ApprovalState:
    task_id: str
    status: str
    created_at: float
    expires_at: float
    decision_by: str | None = None
    decision_at: float | None = None
    reason: str | None = None

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


class ApprovalRegistry:
    """Tracks approval requests with idempotent decision handling."""

    def __init__(self, timeout_sec: int = 600) -> None:
        self.timeout_sec = max(30, timeout_sec)
        self._states: dict[str, ApprovalState] = {}
        self._payloads: dict[str, dict[str, Any]] = {}

    def add(self, task_id: str, payload: dict[str, Any]) -> ApprovalState:
        now = time.time()
        state = ApprovalState(
            task_id=task_id,
            status="PENDING",
            created_at=now,
            expires_at=now + self.timeout_sec,
        )
        self._states[task_id] = state
        self._payloads[task_id] = payload
        return state

    def get_state(self, task_id: str) -> ApprovalState | None:
        state = self._states.get(task_id)
        if not state:
            return None
        if state.status == "PENDING" and time.time() > state.expires_at:
            state.status = "EXPIRED"
        return state

    def get_payload(self, task_id: str) -> dict[str, Any] | None:
        return self._payloads.get(task_id)

    def list_states(self, status: str | None = None) -> list[ApprovalState]:
        wanted = str(status or "").strip().upper()
        items: list[ApprovalState] = []
        for task_id in list(self._states.keys()):
            state = self.get_state(task_id)
            if not state:
                continue
            if wanted and state.status.upper() != wanted:
                continue
            items.append(state)
        items.sort(key=lambda item: item.created_at, reverse=True)
        return items

    def decide(self, task_id: str, approved: bool, actor: str, reason: str | None = None) -> ApprovalState | None:
        state = self.get_state(task_id)
        if not state:
            return None

        # Idempotent: do not overwrite terminal state.
        if state.status in {"APPROVED", "REJECTED", "EXPIRED"}:
            return state

        state.status = "APPROVED" if approved else "REJECTED"
        state.decision_by = actor
        state.decision_at = time.time()
        state.reason = reason
        return state

    def expire(self, task_id: str, actor: str = "system", reason: str = "approval_timeout") -> ApprovalState | None:
        """Mark a pending approval as expired and stamp timeout metadata.

        Safe to call repeatedly; terminal decisions remain unchanged.
        """
        state = self.get_state(task_id)
        if not state:
            return None

        if state.status in {"APPROVED", "REJECTED"}:
            return state

        if state.status == "PENDING":
            state.status = "EXPIRED"

        if state.status == "EXPIRED" and state.decision_at is None:
            state.decision_by = actor
            state.decision_at = time.time()
            state.reason = reason
        return state

    def consume_payload(self, task_id: str) -> dict[str, Any] | None:
        return self._payloads.pop(task_id, None)

    def prune(self) -> int:
        now = time.time()
        removed = 0
        for task_id, state in list(self._states.items()):
            if state.status in {"APPROVED", "REJECTED"} and now - (state.decision_at or now) > 3600:
                self._states.pop(task_id, None)
                self._payloads.pop(task_id, None)
                removed += 1
            elif state.status == "EXPIRED" and now - state.expires_at > 3600:
                self._states.pop(task_id, None)
                self._payloads.pop(task_id, None)
                removed += 1
            elif state.status == "PENDING" and now > state.expires_at:
                state.status = "EXPIRED"
        return removed


def normalize_surface(value: str | None) -> str:
    surface = str(value or "").strip().lower()
    if surface in {"cli", "terminal", "shell"}:
        return "cli"
    if surface in {"api", "web", "http", "app", "browser", "mcp"}:
        return "web"
    if "discord" in surface:
        return "discord"
    return surface or "unknown"


def auto_approved(source_surface: str | None, risk_level: str | None) -> bool:
    """
    Determine whether a task should bypass the manual approval gate.

    Matrix:
      low       -> auto for all surfaces
      medium    -> auto for cli + api/web, hold for discord
      high      -> auto for cli only
      critical  -> hold everywhere

    HEIWA_AUTO_APPROVE=all bypasses all manual approvals.
    HEIWA_AUTO_APPROVE=cli (default) enables the CLI high-risk override.
    """
    mode = str(os.getenv("HEIWA_AUTO_APPROVE", "cli")).strip().lower() or "cli"
    risk = str(risk_level or "low").strip().lower() or "low"
    surface = normalize_surface(source_surface)

    if mode == "all":
        return True
    if risk == "critical":
        return False
    if risk == "low":
        return True
    if risk == "medium":
        return surface in {"cli", "web"}
    if risk == "high":
        return surface == "cli" and mode == "cli"
    return False


_registry: ApprovalRegistry | None = None
_registry_lock = threading.Lock()


def get_approval_registry() -> ApprovalRegistry:
    global _registry
    if _registry is None:
        with _registry_lock:
            if _registry is None:
                timeout_sec = int(os.getenv("HEIWA_APPROVAL_TIMEOUT_SEC", "600") or "600")
                _registry = ApprovalRegistry(timeout_sec=timeout_sec)
    return _registry
