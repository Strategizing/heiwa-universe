import argparse
import json
import os
import sys
import urllib.parse
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
if str(ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT))

from libs.heiwa_sdk.heiwa_net import HeiwaNetProxy

DEFAULT_HUB_URL = os.getenv("HEIWA_HUB_BASE_URL", "http://127.0.0.1:8000")
DEFAULT_AUTH_TOKEN = os.getenv("HEIWA_AUTH_TOKEN", "heiwa-local-dev-token")
_NET_PROXY = HeiwaNetProxy(origin_surface="runtime", agent_id="muscle-console")


class ConsoleError(Exception):
    pass


def print_json(data):
    print(json.dumps(data, indent=2, sort_keys=True))


def http_request(method, base_url, path, token, params=None, body=None):
    method = method.upper()
    url = base_url.rstrip("/") + path
    if params:
        url = url + "?" + urllib.parse.urlencode(params)

    data = None
    headers = {"x-auth-token": token}
    if body is not None:
        data = json.dumps(body).encode("utf-8")
        headers["Content-Type"] = "application/json"

    try:
        resp = _NET_PROXY.request(
            method,
            url,
            data=data,
            headers=headers,
            timeout=30,
            purpose=f"operator console {method} {path}",
            purpose_class="api_data_read" if method == "GET" else "api_data_write",
        )
        resp_body = resp.text or ""
        if resp.status_code >= 400:
            sys.stderr.write(f"HTTP {resp.status_code}: {resp_body}\n")
            raise SystemExit(1)
        if not resp_body:
            return {}
        try:
            return json.loads(resp_body)
        except json.JSONDecodeError:
            return {"raw": resp_body}
    except PermissionError as e:
        sys.stderr.write(f"Net policy denied request: {e}\n")
        raise SystemExit(1)
    except Exception as e:
        sys.stderr.write(f"Request error: {e}\n")
        raise SystemExit(1)


send_request = http_request  # indirection for tests


def cmd_alerts(args):
    if args.alerts_command == "list":
        params = {"status": args.status, "limit": args.limit}
        data = send_request("GET", args.hub_url, "/alerts", args.auth_token, params)
        print_json(data)
    elif args.alerts_command == "ack":
        data = send_request(
            "POST", args.hub_url, f"/alerts/{args.alert_id}/ack", args.auth_token
        )
        print_json(data)
    elif args.alerts_command == "close":
        data = send_request(
            "POST", args.hub_url, f"/alerts/{args.alert_id}/close", args.auth_token
        )
        print_json(data)


def get_proposal(args, proposal_id):
    return send_request(
        "GET", args.hub_url, f"/proposals/{proposal_id}", args.auth_token
    )


def cmd_proposals(args):
    if args.proposals_command == "list":
        params = {}
        if args.status:
            params["status"] = args.status
        if args.limit:
            params["limit"] = args.limit
        data = send_request(
            "GET", args.hub_url, "/proposals", args.auth_token, params=params
        )
        print_json(data)
    elif args.proposals_command == "inspect":
        data = get_proposal(args, args.proposal_id)
        print_json(data)
    elif args.proposals_command == "requeue":
        before = get_proposal(args, args.proposal_id)
        resp = send_request(
            "POST",
            args.hub_url,
            f"/proposals/{args.proposal_id}/requeue",
            args.auth_token,
        )
        after = get_proposal(args, args.proposal_id)
        print_json({"before": before, "requeue": resp, "after": after})


def get_run(args, run_id):
    return send_request("GET", args.hub_url, f"/runs/{run_id}", args.auth_token)


def cmd_runs(args):
    if args.runs_command == "list":
        params = {"limit": args.limit}
        if args.proposal_id:
            params["proposal_id"] = args.proposal_id
        data = send_request(
            "GET", args.hub_url, "/runs", args.auth_token, params=params
        )
        print_json(data)
    elif args.runs_command == "inspect":
        data = get_run(args, args.run_id)
        run = data.get("run", {})

        # If --signals not requested, summary only
        if not args.signals and "signals" in run:
            count = len(run["signals"])
            run["signals"] = f"<{count} signals hidden, use --signals to view>"

        print_json(data)
    elif args.runs_command == "integrity":
        data = send_request(
            "GET",
            args.hub_url,
            f"/runs/{args.run_id}/integrity",
            args.auth_token,
        )
        print_json(data)
    elif args.runs_command == "signals":
        data = send_request(
            "GET", args.hub_url, f"/runs/{args.run_id}/signals", args.auth_token
        )
        if args.json:
            print_json(data)
            return

        print(f"Signals for Run: {data.get('run_id')}")
        signals = data.get("signals", [])
        if not signals:
            print("(No signals)")
        else:
            print(f"{'TIMESTAMP':<25} | {'KIND':<10} | {'MESSAGE'}")
            print("-" * 60)
            for s in signals:
                ts = s.get("timestamp", "")
                kind = s.get("kind", "")
                msg = s.get("msg", "")
                print(f"{ts:<25} | {kind:<10} | {msg}")


def build_parser():
    parser = argparse.ArgumentParser(description="Heiwa Operator Console")
    parser.add_argument("--hub-url", default=DEFAULT_HUB_URL, help="Hub base URL")
    parser.add_argument("--auth-token", default=DEFAULT_AUTH_TOKEN, help="Auth token")
    subparsers = parser.add_subparsers(dest="group", required=True)

    # Alerts
    p_alerts = subparsers.add_parser("alerts", help="Alert commands")
    p_alerts_sub = p_alerts.add_subparsers(dest="alerts_command", required=True)
    p_alerts_list = p_alerts_sub.add_parser("list", help="List alerts")
    p_alerts_list.add_argument("--status", default="OPEN")
    p_alerts_list.add_argument("--limit", type=int, default=50)
    p_alerts_list.set_defaults(func=cmd_alerts)

    p_alerts_ack = p_alerts_sub.add_parser("ack", help="Ack alert")
    p_alerts_ack.add_argument("alert_id")
    p_alerts_ack.set_defaults(func=cmd_alerts)

    p_alerts_close = p_alerts_sub.add_parser("close", help="Close alert")
    p_alerts_close.add_argument("alert_id")
    p_alerts_close.set_defaults(func=cmd_alerts)

    # Proposals
    p_props = subparsers.add_parser("proposals", help="Proposal commands")
    p_props_sub = p_props.add_subparsers(dest="proposals_command", required=True)
    p_props_list = p_props_sub.add_parser("list", help="List proposals")
    p_props_list.add_argument("--status")
    p_props_list.add_argument("--limit", type=int, default=50)
    p_props_list.set_defaults(func=cmd_proposals)

    p_props_inspect = p_props_sub.add_parser("inspect", help="Inspect proposal")
    p_props_inspect.add_argument("proposal_id")
    p_props_inspect.set_defaults(func=cmd_proposals)

    p_props_requeue = p_props_sub.add_parser("requeue", help="Requeue proposal")
    p_props_requeue.add_argument("proposal_id")
    p_props_requeue.set_defaults(func=cmd_proposals)

    # Runs
    p_runs = subparsers.add_parser("runs", help="Run commands")
    p_runs_sub = p_runs.add_subparsers(dest="runs_command", required=True)
    p_runs_list = p_runs_sub.add_parser("list", help="List runs")
    p_runs_list.add_argument("--proposal-id")
    p_runs_list.add_argument("--limit", type=int, default=50)
    p_runs_list.set_defaults(func=cmd_runs)

    p_runs_inspect = p_runs_sub.add_parser("inspect", help="Inspect run")
    p_runs_inspect.add_argument("run_id")
    p_runs_inspect.add_argument("--signals", action="store_true", help="Show signals")
    p_runs_inspect.set_defaults(func=cmd_runs)

    p_runs_integrity = p_runs_sub.add_parser("integrity", help="Run integrity view")
    p_runs_integrity.add_argument("run_id")
    p_runs_integrity.set_defaults(func=cmd_runs)

    p_runs_signals = p_runs_sub.add_parser("signals", help="View run signals")
    p_runs_signals.add_argument("run_id")
    p_runs_signals.add_argument("--json", action="store_true", help="Output raw JSON")
    p_runs_signals.set_defaults(func=cmd_runs)

    # Health
    p_health = subparsers.add_parser("health", help="Operator health dashboard")
    p_health.add_argument("--limit", type=int, default=20)
    p_health.add_argument("--json", action="store_true", help="Output raw JSON")
    p_health.set_defaults(func=cmd_health)

    return parser


def cmd_health(args):
    data = send_request(
        "GET", args.hub_url, "/health/ops", args.auth_token, {"limit": args.limit}
    )
    if args.json:
        print_json(data)
        return

    print("=== OPERATOR HEALTH DASHBOARD ===\n")
    print(f"Snapshot At: {data.get('snapshot_at')}\n")

    print("--- ALERTS SUMMARY ---")
    alerts = data.get("alerts_summary", [])
    if not alerts:
        print("No alerts.")
    else:
        print(f"{'KIND':<25} | {'STATUS':<10} | {'COUNT'}")
        print("-" * 50)
        for a in alerts:
            print(f"{a['kind']:<25} | {a['status']:<10} | {a['count']}")

    print("\n--- CLAIMED PROPOSALS (TTL BOARD) ---")
    claimed = data.get("claimed_proposals", [])
    if not claimed:
        print("No claimed proposals.")
    else:
        print(f"{'PROPOSAL INT':<12} | {'NODE':<36} | {'TTL (s)':<8} | {'HB AGE (s)'}")
        print("-" * 75)
        for p in claimed:
            pid = p["proposal_id"][:12]  # Truncate for display
            node = p["node_id"] or "?"
            ttl = p.get("ttl_seconds_remaining")
            hb = p.get("heartbeat_age_seconds")
            print(f"{pid:<12} | {node:<36} | {str(ttl):<8} | {str(hb)}")

    print("\n--- HEARTBEAT FRESHNESS (STALE FIRST) ---")
    if not claimed:
        print("N/A")
    else:
        # Sort by age desc (stale first)
        stale_sorted = sorted(
            claimed, key=lambda x: x.get("heartbeat_age_seconds") or -1, reverse=True
        )
        # Show top N or all
        top_stale = stale_sorted[: args.limit]
        print(f"{'PROPOSAL INT':<12} | {'NODE':<36} | {'HB AGE (s)'}")
        print("-" * 65)
        for p in top_stale:
            pid = p["proposal_id"][:12]
            node = p["node_id"] or "?"
            hb = p.get("heartbeat_age_seconds")
            print(f"{pid:<12} | {node:<36} | {str(hb)}")
    print("\n=================================")


def main(argv=None):
    parser = build_parser()
    args = parser.parse_args(argv)
    args.func(args)


if __name__ == "__main__":
    main()
