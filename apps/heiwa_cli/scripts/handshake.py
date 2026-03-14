"""
Heiwa Handshake — validate connectivity to the Railway hub.

Checks:
  1. Auth token is set
  2. Hub /health endpoint is reachable
  3. Route probe: send a test classification and verify enrichment works
  4. Local fallback: verify direct execution path works
"""

import json
import os
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
for p in ["packages/heiwa_sdk", "packages/heiwa_protocol", "packages/heiwa_identity", "apps"]:
    p_path = str(ROOT / p)
    if p_path not in sys.path:
        sys.path.insert(0, p_path)

from heiwa_sdk.config import hub_url_candidates, load_swarm_env, settings
load_swarm_env()


def log_step(name: str, ok: bool, detail: str = ""):
    mark = "PASS" if ok else "FAIL"
    print(f"  [{mark}] {name}" + (f" -- {detail}" if detail else ""))


def main():
    print("\n  Heiwa Handshake\n  " + "=" * 40)

    hub_candidates = hub_url_candidates()
    hub_url = hub_candidates[0]
    all_ok = True

    # 1. Auth token
    token = os.getenv("HEIWA_AUTH_TOKEN") or getattr(settings, "HEIWA_AUTH_TOKEN", "") or ""
    ok = bool(token)
    log_step("Auth Token", ok, "set" if token else "HEIWA_AUTH_TOKEN not found")
    if not ok:
        all_ok = False

    # 2. Hub health
    hub_ok = False
    for candidate in hub_candidates:
        try:
            import urllib.request
            req = urllib.request.Request(f"{candidate}/health", method="GET")
            with urllib.request.urlopen(req, timeout=10) as resp:
                data = json.loads(resp.read().decode())
                hub_ok = data.get("status") in {"alive", "OPERATIONAL"}
                backend = data.get("state_backend", "unknown")
                hub_url = candidate
                log_step("Hub Health", hub_ok, f"url={candidate} status={data.get('status')} backend={backend}")
                break
        except Exception as e:
            last_error = str(e)
    if not hub_ok:
        log_step("Hub Health", False, last_error if 'last_error' in locals() else "unreachable")
    if not hub_ok:
        all_ok = False

    # 3. Route probe via hub
    if hub_ok:
        try:
            payload = json.dumps({
                "raw_text": "handshake probe",
                "sender_id": "handshake",
                "source_surface": "cli",
            }).encode()
            req = urllib.request.Request(
                f"{hub_url}/tasks",
                data=payload,
                headers={
                    "Content-Type": "application/json",
                    "Authorization": f"Bearer {token}",
                },
                method="POST",
            )
            with urllib.request.urlopen(req, timeout=10) as resp:
                result = json.loads(resp.read().decode())
                route = result.get("route", {})
                intent = route.get("intent_class", "unknown")
                tool = route.get("target_tool", "unknown")
                log_step("Route Probe", True, f"intent={intent} tool={tool}")
        except Exception as e:
            log_step("Route Probe", False, str(e))
            all_ok = False
    else:
        log_step("Route Probe", False, "skipped (hub unreachable)")

    # 4. Local fallback
    try:
        from heiwa_hub.cognition.enrichment import BrokerEnrichmentService
        from heiwa_protocol.routing import BrokerRouteRequest
        svc = BrokerEnrichmentService()
        req = BrokerRouteRequest(
            request_id="handshake-local",
            task_id="handshake-local",
            raw_text="test local route",
            sender_id="handshake",
            source_surface="cli",
            auth_validated=True,
        )
        result = svc.enrich(req)
        log_step("Local Fallback", True, f"intent={result.intent_class} tool={result.target_tool}")
    except Exception as e:
        log_step("Local Fallback", False, str(e))
        all_ok = False

    print("  " + "=" * 40)

    if not all_ok:
        print("\n  Fix the errors above to restore full Heiwa command path.\n")
    else:
        print("\n  All checks passed.\n")

    return all_ok


if __name__ == "__main__":
    sys.exit(0 if main() else 1)
