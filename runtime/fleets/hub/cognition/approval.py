from __future__ import annotations

import time
from dataclasses import dataclass
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
