"""
Discord Webhook Notifier for Heiwa Hub.

Implements rate limiting (10/min, well under Discord's 30/min limit),
batch aggregation, and deduplication for operational notifications.

See: https://discord.com/safety/using-webhooks-and-embeds
"""

import os
import time
import json
import requests
from datetime import datetime, timezone, timedelta
from typing import Dict, Any, Optional, List
from dataclasses import dataclass, field

try:
    from heiwa_sdk.heiwa_net import HeiwaNetProxy
    _NET_PROXY = HeiwaNetProxy(origin_surface="runtime", agent_id="notifier")
except ImportError:
    _NET_PROXY = None


# Rate limiting state (in-memory, resets on process restart)
@dataclass
class RateLimiter:
    """Simple sliding window rate limiter."""

    max_per_minute: int = 10  # Conservative limit (Discord allows 30)
    window_seconds: int = 60
    timestamps: List[float] = field(default_factory=list)

    def can_send(self) -> bool:
        """Check if we can send a message."""
        self._prune()
        return len(self.timestamps) < self.max_per_minute

    def record_send(self):
        """Record a successful send."""
        self.timestamps.append(time.time())

    def _prune(self):
        """Remove timestamps older than window."""
        cutoff = time.time() - self.window_seconds
        self.timestamps = [t for t in self.timestamps if t > cutoff]

    def remaining(self) -> int:
        """Messages remaining in current window."""
        self._prune()
        return max(0, self.max_per_minute - len(self.timestamps))


# Global rate limiter instance
_rate_limiter = RateLimiter()

# Deduplication set (in-memory, prevents duplicate notifications for same tick)
_sent_alert_ids: set = set()


def reset_dedup():
    """Reset deduplication set. Call at start of each tick."""
    global _sent_alert_ids
    _sent_alert_ids = set()


def format_tick_embed(
    tick_id: str,
    alerts_created: int,
    proposals_created: int,
    proposals_gated: int,
    alert_kinds: Dict[str, int],
    severity: str = "INFO",
    status: str = "OK",
) -> Dict[str, Any]:
    """
    Format a tick summary as a Discord embed.
    """
    # Color based on severity
    if severity == "CRIT":
        color = 0xE74C3C  # Red
        icon = "🔴"
    elif severity == "WARN":
        color = 0xF1C40F  # Orange
        icon = "⚠️"
    else:
        # INFO
        if alerts_created == 0 and status == "OK":
            color = 0x2ECC71  # Green - all quiet
            icon = "✅"
        else:
            color = 0x3498DB  # Blue - standard info
            icon = "ℹ️"

    # Title construction
    if status != "OK":
        title = f"{icon} Hub Tick — {status}"
    elif proposals_created > 0:
        title = f"{icon} Hub Tick — {proposals_created} Proposal(s)"
    elif alerts_created > 0:
        title = f"{icon} Hub Tick — {alerts_created} Alert(s)"
    else:
        title = f"{icon} Hub Tick — All Quiet"

    # Build fields
    fields = [
        {"name": "Alerts", "value": str(alerts_created), "inline": True},
        {"name": "Proposals", "value": str(proposals_created), "inline": True},
        {"name": "Gated", "value": str(proposals_gated), "inline": True},
    ]

    # Alert breakdown if any
    if alert_kinds:
        breakdown = "\n".join([f"• {k}: {v}" for k, v in alert_kinds.items()])
        fields.append({"name": "Alert Types", "value": breakdown, "inline": False})

    embed = {
        "title": title,
        "color": color,
        "fields": fields,
        "footer": {"text": f"Tick ID: {tick_id}"},
        "timestamp": datetime.now(timezone.utc).isoformat(),
    }

    return {"embeds": [embed]}


def format_alert_embed(alert: Dict[str, Any]) -> Dict[str, Any]:
    """
    Format a single alert as a Discord embed.
    Only used for high-priority individual alerts.
    """
    kind = alert.get("kind", "UNKNOWN")
    proposal_id = alert.get("proposal_id", "N/A")
    node_id = alert.get("node_id", "N/A")
    details = alert.get("details_json", {})

    # Color by kind
    colors = {
        "LEASE_EXPIRED": 0xFF6600,  # Orange
        "HEARTBEAT_STALE": 0xFFCC00,  # Yellow
        "PROPOSAL_STUCK_CLAIMED": 0xFF0000,  # Red
        "RUN_FAILURE_SPIKE": 0xFF0000,  # Red
        "SIGNAL_TRUNCATED_SEEN": 0x3498DB,  # Blue
    }
    color = colors.get(kind, 0x808080)

    embed = {
        "title": f"🚨 Alert: {kind}",
        "color": color,
        "fields": [
            {"name": "Proposal", "value": f"`{proposal_id}`", "inline": True},
            {
                "name": "Node",
                "value": f"`{node_id}`" if node_id else "N/A",
                "inline": True,
            },
        ],
        "timestamp": datetime.now(timezone.utc).isoformat(),
    }

    # Add details if present
    if isinstance(details, dict) and details:
        detail_str = json.dumps(details, indent=2)[:500]
        embed["fields"].append(
            {"name": "Details", "value": f"```json\n{detail_str}\n```", "inline": False}
        )

    return {"embeds": [embed]}


def send_notification(
    webhook_url: str,
    payload: Dict[str, Any],
    mode: str = "PRODUCTION",
) -> bool:
    """
    Send a Discord webhook notification.

    Args:
        webhook_url: Discord webhook URL
        payload: Discord webhook payload (with embeds)
        mode: PRODUCTION or SIMULATION

    Returns:
        True if sent (or simulated), False if rate limited or failed
    """
    if not webhook_url:
        print("[NOTIFY] No webhook URL configured, skipping")
        return False

    # Check rate limit
    if not _rate_limiter.can_send():
        print(f"[NOTIFY] Rate limited, {_rate_limiter.remaining()} remaining in window")
        return False

    # Simulation mode - don't actually send
    if mode.upper() == "SIMULATION":
        print(
            f"[NOTIFY] [SIMULATION] Would send: {json.dumps(payload, indent=2)[:500]}..."
        )
        _rate_limiter.record_send()  # Still count for accurate simulation
        return True

    # Actually send
    try:
        if _NET_PROXY:
            response = _NET_PROXY.post(
                webhook_url,
                purpose="discord webhook notification",
                purpose_class="webhook_delivery",
                json=payload,
                timeout=10,
                headers={"Content-Type": "application/json"},
            )
        else:
            response = requests.post(
                webhook_url,
                json=payload,
                timeout=10,
                headers={"Content-Type": "application/json"},
            )

        if response.status_code in [200, 204]:
            _rate_limiter.record_send()
            print(f"[NOTIFY] Sent successfully, {_rate_limiter.remaining()} remaining")
            return True
        else:
            print(
                f"[NOTIFY] Discord returned {response.status_code}: {response.text[:200]}"
            )
            return False

    except requests.exceptions.Timeout:
        print("[NOTIFY] Discord webhook timeout")
        return False
    except requests.exceptions.RequestException as e:
        print(f"[NOTIFY] Discord webhook error: {e}")
        return False


def send_tick_summary(
    webhook_url: str,
    tick_id: str,
    alerts_created: int,
    proposals_created: int,
    proposals_gated: int = 0,
    alert_kinds: Optional[Dict[str, int]] = None,
    mode: str = "PRODUCTION",
    quiet_if_empty: bool = True,
    severity: str = "INFO",
    status: str = "OK",
) -> bool:
    """
    Send a tick summary notification to Discord.
    """
    # Helper to determine if we should skip
    # Skip INFO messages if configured to be quiet and nothing happened
    if quiet_if_empty and severity == "INFO" and alerts_created == 0 and status == "OK":
        print(f"[NOTIFY] Tick {tick_id} quiet (INFO/OK), skipping notification")
        return False

    # Force send if WARN or CRIT, ignoring quiet_if_empty

    payload = format_tick_embed(
        tick_id=tick_id,
        alerts_created=alerts_created,
        proposals_created=proposals_created,
        proposals_gated=proposals_gated,
        alert_kinds=alert_kinds or {},
        severity=severity,
        status=status,
    )

    return send_notification(webhook_url, payload, mode)


# CLI for testing
if __name__ == "__main__":
    import argparse

    parser = argparse.ArgumentParser(description="Test Discord notifier")
    parser.add_argument("--webhook", required=True, help="Discord webhook URL")
    parser.add_argument(
        "--mode", default="SIMULATION", choices=["PRODUCTION", "SIMULATION"]
    )
    args = parser.parse_args()

    # Send a test tick summary
    result = send_tick_summary(
        webhook_url=args.webhook,
        tick_id="TEST-001",
        alerts_created=2,
        proposals_created=1,
        proposals_gated=1,
        alert_kinds={"LEASE_EXPIRED": 1, "HEARTBEAT_STALE": 1},
        mode=args.mode,
    )

    print(f"Result: {'Sent' if result else 'Failed/Skipped'}")