"""
Daily Heartbeat Summary — Report tick health and trends.

Run this once per day (e.g. 09:00 UTC).
"""

import sys
import os
import json
import datetime

from heiwa_sdk.db import db
from heiwa_sdk.config import settings
from heiwa_sdk.notifier import send_notification


def get_stats_embed(hours=24):
    """Generate stats embed."""
    ticks = db.get_recent_ticks(hours)

    total = len(ticks)
    if total == 0:
        return {
            "title": "⚠️ Daily Heartbeat — No Ticks",
            "description": f"No ticks recorded in last {hours}h.",
            "color": 0xF1C40F,  # Orange
            "timestamp": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        }

    success = len([t for t in ticks if t["status"] == "OK"])
    rate = (success / total) * 100

    # Durations (only for OK ticks)
    durations = []
    alerts_total = 0
    proposals_total = 0

    for t in ticks:
        details = t.get("details", {})
        if t["status"] == "OK":
            durations.append(details.get("duration_ms", 0))

        # Count activity regardless of status
        if "steps" in details:
            alerts_total += details["steps"].get("scan_alerts", {}).get("count", 0)
            proposals_total += (
                details["steps"].get("generate_proposals", {}).get("count", 0)
            )
        else:
            # Fallback for old format or failure where details might be flat (unlikely with new schema)
            alerts_total += details.get("alerts_created", 0)
            proposals_total += details.get("proposals_created", 0)

    durations.sort()
    p50 = durations[int(len(durations) * 0.5)] if durations else 0
    p95 = durations[int(len(durations) * 0.95)] if durations else 0

    fields = [
        {"name": "Ticks (24h)", "value": f"{total}", "inline": True},
        {"name": "Success Rate", "value": f"{rate:.1f}%", "inline": True},
        {"name": "Latency", "value": f"p50: {p50}ms\np95: {p95}ms", "inline": True},
        {
            "name": "Activity",
            "value": f"Alerts: {alerts_total}\nProposals: {proposals_total}",
            "inline": False,
        },
    ]

    color = 0x2ECC71  # Green
    if rate < 99.0:
        color = 0xF1C40F  # Orange
    if rate < 90.0:
        color = 0xE74C3C  # Red

    return {
        "title": "💚 Daily Heartbeat",
        "color": color,
        "fields": fields,
        "footer": {"text": f"Heiwa Hub • {settings.HEIWA_ENV}"},
        "timestamp": datetime.datetime.now(datetime.timezone.utc).isoformat(),
    }


def main():
    webhook_url = settings.DISCORD_WEBHOOK_URL
    if not webhook_url:
        print("No webhook URL configured.")
        sys.exit(1)

    print("Generating daily summary...")
    embed = get_stats_embed()

    payload = {"embeds": [embed]}

    # Send
    print(f"Sending to {webhook_url}...")
    success = send_notification(webhook_url, payload, mode=settings.TICK_MODE)

    if success:
        print("Daily heartbeat sent.")
        sys.exit(0)
    else:
        print("Failed to send heartbeat.")
        sys.exit(1)


if __name__ == "__main__":
    main()