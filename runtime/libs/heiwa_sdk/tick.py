"""
Hub Tick Service — Cron-triggered detection and notification loop.

This script is designed to be run by Railway's cron job feature.
It executes a single tick cycle:
1. Scan for alerts (detection engine)
2. Generate proposals from alerts (auto proposal generator)
3. Send notifications (Discord webhook)

Usage:
    python -m services.hub.tick

Environment Variables:
    DATABASE_PATH: Path to SQLite database (default: ./hub.db)
    DISCORD_WEBHOOK_URL: Discord webhook for notifications
    TICK_MODE: 'production' or 'simulation' (default: production)

See: https://docs.railway.com/reference/cron-jobs
"""

import os
import sys
import json
import datetime
import uuid
from pathlib import Path

# Lazy imports to avoid DB connection at import time (Railway build phase issue)
# These are imported inside functions that need them
from libs.heiwa_sdk.config import settings
from libs.heiwa_sdk.notifier import send_tick_summary, reset_dedup


def run_tick() -> dict:
    """
    Execute one tick cycle.

    Returns:
        dict with tick results
    """
    # Lazy import to avoid DB connection during Railway build phase
    from libs.heiwa_sdk.db import db
    import time

    tick_id = f"TICK-{datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%d%H%M%S')}-{uuid.uuid4().hex[:6]}"
    now_utc = datetime.datetime.now(datetime.timezone.utc)

    # Metrics container
    metrics = {
        "tick_id": tick_id,
        "timestamp": now_utc.isoformat(),
        "status": "UNKNOWN",
        "duration_ms": 0,
        "steps": {
            "scan_alerts": {"ms": 0, "count": 0},
            "scan_nodes": {"ms": 0, "alerts": 0},
            "generate_proposals": {"ms": 0, "count": 0},
            "publish_rfcs": {"ms": 0, "count": 0},
            "notify": {"ms": 0, "sent": False},
        },
        "meta": {
            "env": os.getenv("HEIWA_ENV", "production"),
            "tick_mode": settings.TICK_MODE,
        },
        "errors": [],
    }

    print(f"[TICK] Starting {tick_id} at {now_utc.isoformat()}")

    # Reset deduplication for this tick
    reset_dedup()

    # Initialize node_alerts outside the try block for broader scope
    node_alerts = []

    try:
        # Step 1: Scan for alerts
        print("[TICK] Step 1: Scanning for alerts...")
        t0 = time.perf_counter()
        alerts_created = db.scan_alerts(now_utc)
        t1 = time.perf_counter()

        metrics["steps"]["scan_alerts"]["ms"] = int((t1 - t0) * 1000)
        metrics["steps"]["scan_alerts"]["count"] = alerts_created
        print(
            f"[TICK]   Created {alerts_created} new alerts in {metrics['steps']['scan_alerts']['ms']}ms"
        )

    except Exception as e:
        error_msg = f"Scan failed: {e}"
        print(f"[TICK] ERROR: {error_msg}")
        metrics["errors"].append(error_msg)

    try:
        # Step 1b: Scan Nodes (Liveness)
        print("[TICK] Step 1b: Scanning node liveness...")
        t0 = time.perf_counter()
        node_alerts = db.scan_nodes_liveness(
            silent_min=settings.NODE_SILENT_AFTER_MINUTES,
            offline_min=settings.NODE_OFFLINE_AFTER_MINUTES,
        )
        t1 = time.perf_counter()

        metrics["steps"]["scan_nodes"]["ms"] = int((t1 - t0) * 1000)
        metrics["steps"]["scan_nodes"]["alerts"] = len(node_alerts)
        if node_alerts:
            print(f"[TICK]   Created {len(node_alerts)} node alerts: {node_alerts}")

    except Exception as e:
        error_msg = f"Node scan failed: {e}"
        print(f"[TICK] ERROR: {error_msg}")
        metrics["errors"].append(error_msg)

    try:
        # Step 2: Generate proposals from alerts
        print("[TICK] Step 2: Generating proposals from alerts...")
        t0 = time.perf_counter()
        proposals_created = db.generate_proposals_from_alerts()
        t1 = time.perf_counter()

        metrics["steps"]["generate_proposals"]["ms"] = int((t1 - t0) * 1000)
        metrics["steps"]["generate_proposals"]["count"] = proposals_created
        print(
            f"[TICK]   Created {proposals_created} new proposals in {metrics['steps']['generate_proposals']['ms']}ms"
        )

    except Exception as e:
        error_msg = f"Proposal generation failed: {e}"
        print(f"[TICK] ERROR: {error_msg}")
        metrics["errors"].append(error_msg)

    try:
        # Step 2.5: Publish RFCs
        print("[TICK] Step 2.5: Publishing RFCs...")
        t0 = time.perf_counter()
        
        # Lazy import RFC client
        from libs.heiwa_sdk.rfc import rfc_client
        
        open_proposals = db.get_proposals(status="OPEN", limit=10)
        published_count = 0
        
        for p in open_proposals:
            p_id = p["proposal_id"]
            print(f"[TICK]   Publishing RFC for {p_id}...")
            msg_id = rfc_client.post_rfc(p)
            
            if msg_id:
                # Transition to RFC_SENT
                db.transition_proposal_status(
                    p_id, 
                    "RFC_SENT", 
                    {"metadata": json.dumps({"rfc_message_id": msg_id})}
                )
                published_count += 1
            else:
                print(f"[TICK]   Failed to post RFC for {p_id}")

        t1 = time.perf_counter()
        metrics["steps"]["publish_rfcs"] = {"ms": int((t1 - t0) * 1000), "count": published_count}
        if published_count > 0:
            print(f"[TICK]   Published {published_count} RFCs in {metrics['steps']['publish_rfcs']['ms']}ms")

    except Exception as e:
        error_msg = f"RFC publishing failed: {e}"
        print(f"[TICK] ERROR: {error_msg}")
        metrics["errors"].append(error_msg)

    try:
        # Step 3: Send notification
        print("[TICK] Step 3: Sending notification...")
        t0 = time.perf_counter()

        webhook_url = settings.DISCORD_WEBHOOK_URL
        tick_mode = settings.TICK_MODE
        sent = False

        if not webhook_url:
            print("[TICK]   No DISCORD_WEBHOOK_URL configured, skipping notification")
        else:
            # Gather alert kinds for notification summary
            alert_kinds = {}
            try:
                open_alerts = db.get_alerts(status="OPEN", limit=100)
                for alert in open_alerts:
                    kind = alert.get("kind", "UNKNOWN")
                    alert_kinds[kind] = alert_kinds.get(kind, 0) + 1
            except Exception:
                pass

            # Determine severity
            severity = "INFO"

            # WARN criteria
            if metrics["steps"]["generate_proposals"]["count"] > 0:
                severity = "WARN"

            # Node Liveness WARN/CRIT logic
            # If scan_nodes generated alerts, consider severity
            # NODE_OFFLINE -> CRIT, NODE_SILENT -> WARN
            # We don't have the list of specific alerts easily here without querying again or passing it
            # But we have `metrics["steps"]["scan_nodes"]["alerts"]`
            # And `alert_kinds` contains the CURRENT open alerts count.
            # So if `alert_kinds.get("NODE_OFFLINE", 0) > 0`, it might warrant CRIT?
            # Or only if NEW ones created?
            # Directive says: "WARN: node becomes SILENT, CRIT: node becomes OFFLINE".
            # `scan_nodes` logic creates the alerts.
            # If we just created alerts, we should check their type.
            # But `scan_nodes` function returned `alerts_created` (list of kinds).
            # So we can use that local variable `node_alerts`.

            # Wait, local variable `node_alerts` is inside `try` block. Need to access it here.
            # I must initialize it outside try or use metrics.
            # I stored count in metrics, but not kinds.
            # I'll rely on `alert_kinds` map which scans OPEN alerts.

            # I will define `node_alerts` before try blocks

            # Since I am replacing the whole block, I can fix the scope.

            # Re-implementing logic with `node_alerts` check:
            # if "NODE_OFFLINE" in node_alerts: severity = "CRIT"
            # if "NODE_SILENT" in node_alerts: severity = "WARN"

            # Check newly created node alerts for severity escalation
            for alert_kind in node_alerts:
                if alert_kind == "NODE_OFFLINE":
                    severity = "CRIT"  # If any new NODE_OFFLINE alert, it's critical
                    break  # CRIT is highest, no need to check further
                elif alert_kind == "NODE_SILENT" and severity != "CRIT":
                    severity = "WARN"  # If new NODE_SILENT and not already CRIT

            # Slow tick (WARN)
            current_duration = (
                datetime.datetime.now(datetime.timezone.utc) - now_utc
            ).total_seconds()
            if current_duration > 5.0 and severity != "CRIT":  # Don't downgrade CRIT
                severity = "WARN"

            # CRIT criteria (errors in metrics so far)
            if metrics["errors"]:
                severity = "CRIT"

            sent = send_tick_summary(
                webhook_url=webhook_url,
                tick_id=tick_id,
                alerts_created=metrics["steps"]["scan_alerts"]["count"]
                + metrics["steps"]["scan_nodes"]["alerts"],  # Sum them?
                # Wait, `alerts_created` argument for `send_tick_summary` is "Number of alerts created THIS TICK".
                # `db.scan_alerts` returns count.
                # `db.scan_nodes` returns list of alerts.
                # Total = scan_alerts + len(scan_nodes).
                proposals_created=metrics["steps"]["generate_proposals"]["count"],
                proposals_gated=0,
                alert_kinds=alert_kinds,
                mode=tick_mode,
                quiet_if_empty=True,
                severity=severity,
                status="FAIL" if metrics["errors"] else "OK",
            )

            if sent:
                print("[TICK]   Notification sent")
            else:
                print("[TICK]   Notification skipped (quiet or rate limited)")

        t1 = time.perf_counter()
        metrics["steps"]["notify"]["ms"] = int((t1 - t0) * 1000)
        metrics["steps"]["notify"]["sent"] = sent

    except Exception as e:
        error_msg = f"Notification failed: {e}"
        print(f"[TICK] ERROR: {error_msg}")
        metrics["errors"].append(error_msg)

    # Complete
    ended_at = datetime.datetime.now(datetime.timezone.utc)
    metrics["duration_ms"] = int((ended_at - now_utc).total_seconds() * 1000)
    metrics["status"] = "OK" if not metrics["errors"] else "FAIL"

    print(
        f"[TICK] Completed {tick_id} in {metrics['duration_ms']}ms — {metrics['status']}"
    )

    # Persist to DB (Persistence Phase)
    # Mapping metrics to DB schema format where needed, or storing full blob
    try:
        # We store the structured metrics in details_json
        tick_record = {
            "tick_id": tick_id,
            "started_at": metrics["timestamp"],
            "ended_at": ended_at.isoformat(),
            "status": metrics["status"],
            "details_json": metrics,  # Store full metrics blob
        }
        db.record_tick(tick_record)
    except Exception as e:
        print(f"[TICK] Warning: Failed to record tick to DB: {e}")
        # In strict fail-closed mode, we might want to fail the tick here,
        # but for now we log it. The directive says "Failure to persist -> tick FAIL (fail-closed)."
        # So let's respect that.
        metrics["status"] = "FAIL"
        metrics["errors"].append(f"Persistence failed: {e}")
        print(f"[TICK] CRITICAL: DB Persistence failed, marking tick as FAIL")

    return metrics


def router_tick() -> dict:
    """
    Phase 2 Router Tick: Route APPROVED/QUEUED proposals to eligible nodes.

    Behavior:
    - Scan APPROVED/QUEUED proposals that haven't expired
    - For each, find eligible nodes based on execution_targeting
    - If eligible node available → ASSIGNED
    - If no eligible nodes + QUEUE policy → QUEUED
    - If no eligible nodes + EXPIRE policy → EXPIRED

    Returns:
        dict with routing results
    """
    from libs.heiwa_sdk.db import db
    from libs.heiwa_sdk.config import settings
    import hashlib
    import json as _json

    if not settings.PHASE2_ROUTER_ENABLED:
        print("[ROUTER] Router disabled by feature flag")
        return {"status": "disabled", "routed": 0, "queued": 0, "expired": 0}

    now = datetime.datetime.now(datetime.timezone.utc)
    now_iso = now.isoformat()

    result = {
        "status": "OK",
        "routed": 0,
        "queued": 0,
        "expired": 0,
        "errors": [],
    }

    print(f"[ROUTER] Starting router tick at {now_iso}")

    try:
        proposals = db.get_routable_proposals()
        print(f"[ROUTER] Found {len(proposals)} routable proposals")

        for p in proposals:
            proposal_id = p["proposal_id"]

            # Parse execution_targeting
            targeting = {}
            try:
                targeting_raw = p.get("execution_targeting") or "{}"
                targeting = (
                    _json.loads(targeting_raw)
                    if isinstance(targeting_raw, str)
                    else targeting_raw
                )
            except:
                pass

            requires = targeting.get("requires", [])
            privilege_tier = targeting.get("privilege_tier", "cloud_safe")
            policy = targeting.get("policy", "QUEUE")  # QUEUE, REROUTE, EXPIRE

            # Find eligible nodes
            eligible = db.get_eligible_nodes(requires, privilege_tier)

            if eligible:
                # Pick first eligible node (could add scoring later)
                node = eligible[0]
                node_id = node["node_id"]

                # Assignment TTL: 15 minutes (or from targeting)
                assignment_ttl = targeting.get("assignment_ttl_seconds", 900)
                assignment_expires = (
                    now + datetime.timedelta(seconds=assignment_ttl)
                ).isoformat()

                # Compute hub_signature (signing the proposal for node verification)
                payload_str = p.get("payload") or ""
                proposal_hash = (
                    p.get("proposal_hash")
                    or hashlib.sha256(payload_str.encode()).hexdigest()
                )
                # In production, this would be a proper cryptographic signature
                hub_signature = f"SIG-{proposal_hash[:16]}"

                # Increment attempt count
                attempt_count = (p.get("attempt_count") or 0) + 1

                # Update proposal
                success = db.transition_proposal_status(
                    proposal_id,
                    "ASSIGNED",
                    {
                        "assigned_node_id": node_id,
                        "assignment_expires_at": assignment_expires,
                        "hub_signature": hub_signature,
                        "attempt_count": attempt_count,
                        "eligibility_snapshot": _json.dumps(
                            {
                                "eligible_count": len(eligible),
                                "assigned_to": node_id,
                                "timestamp": now_iso,
                            }
                        ),
                    },
                )

                if success:
                    print(f"[ROUTER] Assigned {proposal_id} to {node_id}")
                    result["routed"] += 1
                else:
                    result["errors"].append(f"Failed to assign {proposal_id}")

            else:
                # No eligible nodes
                if policy == "EXPIRE":
                    db.expire_proposal(proposal_id, "no_eligible_nodes")
                    print(f"[ROUTER] Expired {proposal_id} (EXPIRE policy, no nodes)")
                    result["expired"] += 1
                else:
                    # QUEUE or REROUTE policy - keep in queue
                    if p.get("status") != "QUEUED":
                        db.transition_proposal_status(proposal_id, "QUEUED")
                        print(
                            f"[ROUTER] Queued {proposal_id} (waiting for eligible node)"
                        )
                    result["queued"] += 1

    except Exception as e:
        error_msg = f"Router error: {e}"
        print(f"[ROUTER] ERROR: {error_msg}")
        result["errors"].append(error_msg)
        result["status"] = "FAIL"

    print(
        f"[ROUTER] Complete: routed={result['routed']}, queued={result['queued']}, expired={result['expired']}"
    )
    return result


def main():
    """Main entry point for CLI/cron execution."""
    import argparse

    parser = argparse.ArgumentParser(description="Run Hub tick cycle")
    parser.add_argument("--json", action="store_true", help="Output result as JSON")
    parser.add_argument(
        "--once",
        action="store_true",
        help="Run once and exit (default behavior, explicit flag for smoke tests)",
    )
    parser.add_argument(
        "--dry-run", action="store_true", help="Alias for TICK_MODE=simulation"
    )
    parser.add_argument(
        "--notify-test", action="store_true", help="Send a test notification to Discord"
    )

    args = parser.parse_args()

    # Override mode if --dry-run
    if args.dry_run:
        os.environ["TICK_MODE"] = "simulation"
        # Reload settings
        settings.TICK_MODE = "simulation"

    # Handle --notify-test
    if args.notify_test:
        from libs.heiwa_sdk.notifier import send_notification

        webhook_url = settings.DISCORD_WEBHOOK_URL
        if not webhook_url:
            print("Error: DISCORD_WEBHOOK_URL not set.")
            sys.exit(1)

        print(f"Sending test notification to {webhook_url}...")
        try:
            success = send_notification(
                webhook_url,
                {
                    "content": "🔔 **Heiwa Hub Tick Notification Test**\nPipeline discipline check engaged.",
                    "username": "Heiwa Hub",
                },
                mode=settings.TICK_MODE,
            )
            if success:
                print("Test notification sent successfully.")
                sys.exit(0)
            else:
                print("Failed to send test notification.")
                sys.exit(1)
        except Exception as e:
            print(f"Exception during notification test: {e}")
            sys.exit(1)

    result = run_tick()

    # Always emit a structured log line (JSON) for observability
    structured_log = json.dumps(result)
    print(f"TICK_METRICS_JSON={structured_log}")

    if args.json:
        # If user specifically asked for JSON output only
        print(json.dumps(result, indent=2))
    else:
        # Human-readable summary
        print(f"\n=== TICK SUMMARY ===")
        print(f"ID: {result['tick_id']}")
        print(f"Alerts: {result['steps']['scan_alerts']['count']}")
        print(f"Proposals: {result['steps']['generate_proposals']['count']}")
        print(f"Notified: {result['steps']['notify']['sent']}")
        if result["errors"]:
            print(f"Errors: {len(result['errors'])}")
            for e in result["errors"]:
                print(f"  - {e}")
        print(f"Duration: {result['duration_ms']}ms")

    # Exit with error code if any errors
    if result["errors"]:
        sys.exit(1)
    sys.exit(0)


if __name__ == "__main__":
    main()
