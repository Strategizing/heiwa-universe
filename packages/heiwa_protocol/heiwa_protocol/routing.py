from __future__ import annotations

from dataclasses import asdict, dataclass, field
import time
from typing import Any


BROKER_ENVELOPE_VERSION = "2026-03-13"
PRIVACY_LEVELS = {"local", "sensitive", "sovereign"}


def normalize_privacy_level(value: str | None, raw_text: str = "") -> str:
    candidate = (value or "").strip().lower()
    if candidate in PRIVACY_LEVELS:
        return candidate

    lowered = (raw_text or "").lower()
    sovereign_markers = (
        "sovereign",
        "local only",
        "keep local",
        "keep it local",
        "on-device",
        "on device",
        "private only",
        "no cloud",
    )
    if any(marker in lowered for marker in sovereign_markers):
        return "sovereign"
    return "local"


@dataclass(slots=True)
class BrokerRouteRequest:
    request_id: str
    task_id: str
    raw_text: str
    sender_id: str = ""
    source_surface: str = "cli"
    response_channel_id: str | int | None = None
    response_thread_id: str | int | None = None
    auth_validated: bool = False
    timestamp: float = field(default_factory=time.time)
    envelope_version: str = BROKER_ENVELOPE_VERSION
    privacy_level: str | None = None

    @classmethod
    def from_payload(cls, payload: dict[str, Any]) -> "BrokerRouteRequest":
        return cls(
            request_id=str(payload.get("request_id") or ""),
            task_id=str(payload.get("task_id") or ""),
            raw_text=str(payload.get("raw_text") or ""),
            sender_id=str(payload.get("sender_id") or ""),
            source_surface=str(payload.get("source_surface") or "cli"),
            response_channel_id=payload.get("response_channel_id"),
            response_thread_id=payload.get("response_thread_id"),
            auth_validated=bool(payload.get("auth_validated")),
            timestamp=float(payload.get("timestamp") or time.time()),
            envelope_version=str(payload.get("envelope_version") or BROKER_ENVELOPE_VERSION),
            privacy_level=normalize_privacy_level(payload.get("privacy_level"), str(payload.get("raw_text") or "")),
        )

    def to_dict(self) -> dict[str, Any]:
        payload = asdict(self)
        payload["privacy_level"] = normalize_privacy_level(self.privacy_level, self.raw_text)
        return payload


@dataclass(slots=True)
class BrokerRouteResult:
    request_id: str
    task_id: str
    envelope_version: str
    raw_text: str
    source_surface: str
    intent_class: str
    risk_level: str
    privacy_level: str
    compute_class: int
    assigned_worker: str
    target_tool: str
    target_model: str
    target_runtime: str
    target_tier: str
    requires_approval: bool
    rationale: str
    confidence: float = 0.0
    escalation_reasons: list[str] = field(default_factory=list)
    assumptions: list[str] = field(default_factory=list)
    missing_details: list[str] = field(default_factory=list)
    normalization: dict[str, Any] = field(default_factory=dict)
    gateway_transport: str = "websocket"
    error: str | None = None
    message: str | None = None

    @classmethod
    def from_payload(cls, payload: dict[str, Any]) -> "BrokerRouteResult":
        return cls(
            request_id=str(payload.get("request_id") or ""),
            task_id=str(payload.get("task_id") or ""),
            envelope_version=str(payload.get("envelope_version") or BROKER_ENVELOPE_VERSION),
            raw_text=str(payload.get("raw_text") or ""),
            source_surface=str(payload.get("source_surface") or "cli"),
            intent_class=str(payload.get("intent_class") or "general"),
            risk_level=str(payload.get("risk_level") or "low"),
            privacy_level=normalize_privacy_level(payload.get("privacy_level"), str(payload.get("raw_text") or "")),
            compute_class=int(payload.get("compute_class") or 1),
            assigned_worker=str(payload.get("assigned_worker") or ""),
            target_tool=str(payload.get("target_tool") or "heiwa_claw"),
            target_model=str(payload.get("target_model") or ""),
            target_runtime=str(payload.get("target_runtime") or "railway"),
            target_tier=str(payload.get("target_tier") or "tier1_local"),
            requires_approval=bool(payload.get("requires_approval")),
            rationale=str(payload.get("rationale") or ""),
            confidence=float(payload.get("confidence") or 0.0),
            escalation_reasons=list(payload.get("escalation_reasons") or []),
            assumptions=list(payload.get("assumptions") or []),
            missing_details=list(payload.get("missing_details") or []),
            normalization=dict(payload.get("normalization") or {}),
            gateway_transport=str(payload.get("gateway_transport") or "websocket"),
            error=payload.get("error"),
            message=payload.get("message"),
        )

    def to_dict(self) -> dict[str, Any]:
        payload = asdict(self)
        payload["privacy_level"] = normalize_privacy_level(self.privacy_level, self.raw_text)
        return payload
