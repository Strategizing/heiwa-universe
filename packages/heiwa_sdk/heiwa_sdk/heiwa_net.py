"""
heiwa_net — Controlled internet access proxy for Heiwa runtime/agents.

All automated internet traffic should be routed through this module.
It implements the heiwa_net_request_v2 / heiwa_net_decision_v2 / heiwa_net_result_v2
contract triad, evaluating each request against the net policy configuration
before allowing execution.

Usage:
    from heiwa_sdk.heiwa_net import HeiwaNetProxy

    proxy = HeiwaNetProxy()
    response = proxy.get("https://api.github.com/repos/...", purpose="fetch repo metadata")
    # or
    response = await proxy.async_get("https://...", purpose="health check")
"""
from __future__ import annotations

import hashlib
import json
import logging
import os
import time
import uuid
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any
from urllib.parse import urlparse

logger = logging.getLogger("heiwa.net")

# ── Constants ──────────────────────────────────────────────────────────

HEIWA_HOME = Path(os.environ.get("HEIWA_HOME", str(Path.home() / ".heiwa")))
NET_POLICY_PATH = HEIWA_HOME / "policy" / "internet" / "net_policy_v2.json"
NET_AUDIT_DIR = HEIWA_HOME / "state" / "net" / "audit"

PURPOSE_CLASSES = {
    "api_auth", "api_data_read", "api_data_write", "api_deploy",
    "webhook_delivery", "health_check", "model_inference", "search",
    "scrape", "file_download", "dns_lookup", "messaging", "other",
}

RISK_CLASSES = {"low", "medium", "high", "critical"}


# ── Data structures ───────────────────────────────────────────────────

@dataclass
class NetRequest:
    """heiwa_net_request_v2 envelope."""
    url: str
    method: str = "GET"
    purpose: str = ""
    purpose_class: str = "other"
    risk_class: str = "medium"
    origin_surface: str = "runtime"
    agent_id: str | None = None
    device_id: str | None = None
    tenant_id: str | None = None
    headers_redacted: bool = True
    body_hash: str | None = None
    timeout_ms: int | None = None
    retry_policy: str | None = None
    request_id: str = field(default_factory=lambda: f"nr_{uuid.uuid4().hex}")
    created_at: str = field(default_factory=lambda: _iso_now())

    def to_envelope(self) -> dict[str, Any]:
        parsed = urlparse(self.url)
        return {
            "schema_version": "heiwa_net_request_v2",
            "request_id": self.request_id,
            "created_at": self.created_at,
            "origin": {
                "surface": self.origin_surface,
                "agent_id": self.agent_id,
                "device_id": self.device_id,
                "tenant_id": self.tenant_id,
            },
            "destination": {
                "url": self.url,
                "host": parsed.hostname,
                "port": parsed.port,
                "protocol": parsed.scheme,
            },
            "method": self.method,
            "purpose": self.purpose,
            "purpose_class": self.purpose_class,
            "risk_class": self.risk_class,
            "requires_approval": False,  # Set by policy engine
            "headers_redacted": self.headers_redacted,
            "body_hash": self.body_hash,
            "timeout_ms": self.timeout_ms,
            "retry_policy": self.retry_policy,
        }


@dataclass
class NetDecision:
    """heiwa_net_decision_v2 envelope."""
    request_id: str
    decision: str  # allow, deny, approval_required, rate_limited, redirect
    reason: str
    matched_rule: str | None = None
    decision_id: str = field(default_factory=lambda: f"nd_{uuid.uuid4().hex}")
    decided_at: str = field(default_factory=lambda: _iso_now())

    def to_envelope(self) -> dict[str, Any]:
        return {
            "schema_version": "heiwa_net_decision_v2",
            "decision_id": self.decision_id,
            "request_id": self.request_id,
            "decided_at": self.decided_at,
            "decision": self.decision,
            "reason": self.reason,
            "matched_rule": self.matched_rule,
        }


@dataclass
class NetResult:
    """heiwa_net_result_v2 envelope."""
    request_id: str
    decision_id: str
    status: str  # success, error, timeout, denied_at_execution
    duration_ms: int = 0
    http_status: int | None = None
    response_size_bytes: int | None = None
    error_summary: str | None = None
    result_id: str = field(default_factory=lambda: f"nres_{uuid.uuid4().hex}")
    completed_at: str = field(default_factory=lambda: _iso_now())

    def to_envelope(self) -> dict[str, Any]:
        return {
            "schema_version": "heiwa_net_result_v2",
            "result_id": self.result_id,
            "request_id": self.request_id,
            "decision_id": self.decision_id,
            "completed_at": self.completed_at,
            "status": self.status,
            "http_status": self.http_status,
            "duration_ms": self.duration_ms,
            "response_size_bytes": self.response_size_bytes,
            "error_summary": self.error_summary,
            "redaction_applied": True,
        }


# ── Policy engine ─────────────────────────────────────────────────────

class NetPolicyEngine:
    """Evaluates net requests against the heiwa net policy configuration."""

    def __init__(self, policy_path: Path | None = None):
        self._policy_path = policy_path or NET_POLICY_PATH
        self._policy: dict[str, Any] | None = None
        self._load_ts: float = 0

    def _load(self) -> dict[str, Any]:
        now = time.monotonic()
        if self._policy and (now - self._load_ts) < 30:
            return self._policy
        if not self._policy_path.exists():
            logger.warning("Net policy not found at %s — defaulting to deny-all", self._policy_path)
            self._policy = {"default_decision": "deny", "rules": []}
            self._load_ts = now
            return self._policy
        try:
            self._policy = json.loads(self._policy_path.read_text(encoding="utf-8"))
            self._load_ts = now
        except Exception as exc:
            logger.error("Failed to load net policy: %s", exc)
            self._policy = {"default_decision": "deny", "rules": []}
            self._load_ts = now
        return self._policy

    def evaluate(self, request: NetRequest) -> NetDecision:
        policy = self._load()
        envelope = request.to_envelope()
        dest = envelope.get("destination", {})
        host = dest.get("host", "")
        port = dest.get("port")
        protocol = dest.get("protocol", "")
        method = envelope.get("method", "")
        purpose_class = envelope.get("purpose_class", "other")

        for rule in policy.get("rules", []):
            rule_id = rule.get("rule_id", "unknown")
            match_spec = rule.get("match", {})

            if not self._matches_rule(match_spec, host=host, port=port, protocol=protocol,
                                       method=method, purpose_class=purpose_class):
                continue

            decision_str = rule.get("decision", "deny")

            # Check requires_approval_if_write
            if rule.get("requires_approval_if_write") and method in ("POST", "PUT", "PATCH", "DELETE"):
                decision_str = "approval_required"

            return NetDecision(
                request_id=request.request_id,
                decision=decision_str,
                reason=rule.get("description", rule_id),
                matched_rule=rule_id,
            )

        return NetDecision(
            request_id=request.request_id,
            decision=policy.get("default_decision", "deny"),
            reason="No matching rule — default policy applied",
            matched_rule=None,
        )

    def _matches_rule(self, match: dict[str, Any], *, host: str, port: int | None,
                      protocol: str, method: str, purpose_class: str) -> bool:
        if not match:
            return True  # Empty match = catch-all

        # Check purpose_class
        if "purpose_class" in match:
            if purpose_class not in match["purpose_class"]:
                return False

        # Check method
        if "method" in match:
            if method not in match["method"]:
                return False

        # Check protocol
        if "protocol" in match:
            if protocol not in match["protocol"]:
                return False

        # Check destination_port
        if "destination_port" in match:
            if port not in match["destination_port"]:
                return False

        # Check destination_host_pattern
        if "destination_host_pattern" in match:
            patterns = match["destination_host_pattern"]
            if not any(self._host_matches(host, p) for p in patterns):
                return False

        return True

    @staticmethod
    def _host_matches(host: str, pattern: str) -> bool:
        if not host or not pattern:
            return False
        if pattern == host:
            return True
        if pattern.startswith("*.") and host.endswith(pattern[1:]):
            return True
        if pattern.endswith(".*.*.*"):
            # IP prefix pattern like "100.*.*.*"
            prefix = pattern.split(".")[0]
            return host.startswith(f"{prefix}.")
        return False


# ── Audit logger ──────────────────────────────────────────────────────

class NetAuditLogger:
    """Logs net request/decision/result triad to the audit directory."""

    def __init__(self, audit_dir: Path | None = None):
        self._audit_dir = audit_dir or NET_AUDIT_DIR

    def log(self, request: NetRequest, decision: NetDecision, result: NetResult | None = None) -> None:
        day = datetime.now(timezone.utc).strftime("%Y-%m-%d")
        day_dir = self._audit_dir / day
        day_dir.mkdir(parents=True, exist_ok=True)

        entry = {
            "logged_at": _iso_now(),
            "request": request.to_envelope(),
            "decision": decision.to_envelope(),
        }
        if result:
            entry["result"] = result.to_envelope()

        log_file = day_dir / "net_audit.jsonl"
        line = json.dumps(entry, separators=(",", ":"), ensure_ascii=True) + "\n"
        with log_file.open("a", encoding="utf-8") as fh:
            fh.write(line)


# ── Proxy (main interface) ────────────────────────────────────────────

class HeiwaNetProxy:
    """
    Drop-in replacement for requests.get/post/etc that enforces Heiwa net policy.

    Usage:
        proxy = HeiwaNetProxy(origin_surface="agent", agent_id="spine")
        resp = proxy.get("https://api.github.com/...", purpose="fetch CI status")
    """

    def __init__(
        self,
        *,
        origin_surface: str = "runtime",
        agent_id: str | None = None,
        device_id: str | None = None,
        tenant_id: str | None = None,
        policy_path: Path | None = None,
        audit_dir: Path | None = None,
        dry_run: bool = False,
    ):
        self.origin_surface = origin_surface
        self.agent_id = agent_id
        self.device_id = device_id
        self.tenant_id = tenant_id
        self.dry_run = dry_run
        self._engine = NetPolicyEngine(policy_path)
        self._audit = NetAuditLogger(audit_dir)

    def _build_request(
        self, method: str, url: str, *,
        purpose: str = "",
        purpose_class: str = "other",
        risk_class: str = "medium",
        body: bytes | None = None,
        timeout_ms: int | None = None,
    ) -> NetRequest:
        return NetRequest(
            url=url,
            method=method,
            purpose=purpose,
            purpose_class=purpose_class,
            risk_class=risk_class,
            origin_surface=self.origin_surface,
            agent_id=self.agent_id,
            device_id=self.device_id,
            tenant_id=self.tenant_id,
            body_hash=hashlib.sha256(body).hexdigest() if body else None,
            timeout_ms=timeout_ms,
        )

    def request(
        self, method: str, url: str, *,
        purpose: str = "",
        purpose_class: str = "other",
        risk_class: str = "medium",
        timeout: int | None = None,
        **kwargs: Any,
    ) -> Any:
        """Execute a policy-gated HTTP request (synchronous)."""
        body = kwargs.get("data") or kwargs.get("json")
        body_bytes = json.dumps(body).encode() if isinstance(body, (dict, list)) else (body if isinstance(body, bytes) else None)

        nr = self._build_request(
            method, url,
            purpose=purpose,
            purpose_class=purpose_class,
            risk_class=risk_class,
            body=body_bytes,
            timeout_ms=timeout * 1000 if timeout else None,
        )

        decision = self._engine.evaluate(nr)
        logger.info("Net policy: %s %s → %s (%s)", method, url, decision.decision, decision.reason)

        if decision.decision != "allow":
            self._audit.log(nr, decision)
            if decision.decision == "approval_required":
                raise PermissionError(
                    f"Heiwa net policy requires approval: {decision.reason} "
                    f"(rule: {decision.matched_rule}, request: {nr.request_id})"
                )
            raise PermissionError(
                f"Heiwa net policy denied: {decision.reason} "
                f"(rule: {decision.matched_rule}, request: {nr.request_id})"
            )

        if self.dry_run:
            self._audit.log(nr, decision)
            return None

        start = time.monotonic()
        import requests as _requests
        try:
            resp = _requests.request(method, url, timeout=timeout, **kwargs)
            elapsed_ms = int((time.monotonic() - start) * 1000)

            result = NetResult(
                request_id=nr.request_id,
                decision_id=decision.decision_id,
                status="success",
                duration_ms=elapsed_ms,
                http_status=resp.status_code,
                response_size_bytes=len(resp.content) if resp.content else 0,
            )
            self._audit.log(nr, decision, result)
            return resp

        except Exception as exc:
            elapsed_ms = int((time.monotonic() - start) * 1000)
            is_timeout = "timeout" in str(exc).lower() or "timed out" in str(exc).lower()
            result = NetResult(
                request_id=nr.request_id,
                decision_id=decision.decision_id,
                status="timeout" if is_timeout else "error",
                duration_ms=elapsed_ms,
                error_summary=str(exc)[:500],
            )
            self._audit.log(nr, decision, result)
            raise

    def get(self, url: str, *, purpose: str = "", purpose_class: str = "api_data_read", **kwargs: Any) -> Any:
        return self.request("GET", url, purpose=purpose, purpose_class=purpose_class, risk_class="low", **kwargs)

    def post(self, url: str, *, purpose: str = "", purpose_class: str = "api_data_write", **kwargs: Any) -> Any:
        return self.request("POST", url, purpose=purpose, purpose_class=purpose_class, risk_class="medium", **kwargs)

    def put(self, url: str, *, purpose: str = "", purpose_class: str = "api_data_write", **kwargs: Any) -> Any:
        return self.request("PUT", url, purpose=purpose, purpose_class=purpose_class, risk_class="medium", **kwargs)

    def delete(self, url: str, *, purpose: str = "", purpose_class: str = "api_data_write", **kwargs: Any) -> Any:
        return self.request("DELETE", url, purpose=purpose, purpose_class=purpose_class, risk_class="high", **kwargs)


# ── Async variant ─────────────────────────────────────────────────────

@dataclass
class HeiwaBufferedAsyncResponse:
    """Buffered async response returned after the aiohttp session has closed."""
    status: int
    headers: dict[str, str]
    body: bytes
    url: str

    @property
    def content(self) -> bytes:
        return self.body

    @property
    def status_code(self) -> int:
        return self.status

    def text(self, encoding: str = "utf-8") -> str:
        return self.body.decode(encoding, errors="replace")

    def json(self) -> Any:
        if not self.body:
            return None
        return json.loads(self.body.decode("utf-8"))

class HeiwaAsyncNetProxy:
    """
    Async variant of HeiwaNetProxy using aiohttp.

    Usage:
        proxy = HeiwaAsyncNetProxy(origin_surface="agent", agent_id="spine")
        resp = await proxy.get("https://...", purpose="health check")
    """

    def __init__(self, *, origin_surface: str = "runtime", agent_id: str | None = None,
                 device_id: str | None = None, tenant_id: str | None = None,
                 policy_path: Path | None = None, audit_dir: Path | None = None,
                 dry_run: bool = False):
        self.origin_surface = origin_surface
        self.agent_id = agent_id
        self.device_id = device_id
        self.tenant_id = tenant_id
        self.dry_run = dry_run
        self._engine = NetPolicyEngine(policy_path)
        self._audit = NetAuditLogger(audit_dir)

    async def request(self, method: str, url: str, *, purpose: str = "",
                      purpose_class: str = "other", risk_class: str = "medium",
                      timeout: int | None = None, **kwargs: Any) -> Any:
        import aiohttp

        nr = NetRequest(
            url=url, method=method, purpose=purpose,
            purpose_class=purpose_class, risk_class=risk_class,
            origin_surface=self.origin_surface, agent_id=self.agent_id,
            device_id=self.device_id, tenant_id=self.tenant_id,
        )

        decision = self._engine.evaluate(nr)
        logger.info("Net policy (async): %s %s → %s", method, url, decision.decision)

        if decision.decision != "allow":
            self._audit.log(nr, decision)
            raise PermissionError(
                f"Heiwa net policy: {decision.decision} — {decision.reason} "
                f"(rule: {decision.matched_rule})"
            )

        if self.dry_run:
            self._audit.log(nr, decision)
            return None

        start = time.monotonic()
        ct = aiohttp.ClientTimeout(total=timeout) if timeout else None
        try:
            async with aiohttp.ClientSession(timeout=ct) as session:
                async with session.request(method, url, **kwargs) as resp:
                    body = await resp.read()
                    elapsed_ms = int((time.monotonic() - start) * 1000)
                    result = NetResult(
                        request_id=nr.request_id,
                        decision_id=decision.decision_id,
                        status="success",
                        duration_ms=elapsed_ms,
                        http_status=resp.status,
                        response_size_bytes=len(body),
                    )
                    self._audit.log(nr, decision, result)
                    return HeiwaBufferedAsyncResponse(
                        status=resp.status,
                        headers=dict(resp.headers),
                        body=body,
                        url=str(resp.url),
                    )
        except Exception as exc:
            elapsed_ms = int((time.monotonic() - start) * 1000)
            result = NetResult(
                request_id=nr.request_id,
                decision_id=decision.decision_id,
                status="error",
                duration_ms=elapsed_ms,
                error_summary=str(exc)[:500],
            )
            self._audit.log(nr, decision, result)
            raise

    async def get(self, url: str, *, purpose: str = "", purpose_class: str = "api_data_read", **kwargs: Any) -> Any:
        return await self.request("GET", url, purpose=purpose, purpose_class=purpose_class, risk_class="low", **kwargs)

    async def post(self, url: str, *, purpose: str = "", purpose_class: str = "api_data_write", **kwargs: Any) -> Any:
        return await self.request("POST", url, purpose=purpose, purpose_class=purpose_class, risk_class="medium", **kwargs)


# ── Helpers ───────────────────────────────────────────────────────────

def _iso_now() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")