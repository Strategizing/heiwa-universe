#!/usr/bin/env python3
"""
Probe Heiwa Domains based on bootstrap manifest.
Runs DNS resolution and HTTP health checks.
"""

import json
import socket
import sys
from pathlib import Path
from urllib.error import HTTPError
from urllib.request import urlopen

ROOT = Path(__file__).resolve().parents[4]
MANIFEST_PATH = ROOT / "infrastructure" / "domains" / "heiwa-ltd.bootstrap.json"

def resolve_dns(host: str) -> bool:
    try:
        socket.gethostbyname(host)
        return True
    except socket.gaierror:
        return False

def http_health(url: str, timeout: int = 5) -> tuple[bool, str]:
    try:
        with urlopen(url, timeout=timeout) as resp:
            code = getattr(resp, "status", 200)
            return (200 <= code < 300, f"HTTP {code}")
    except HTTPError as exc:
        if exc.code in {401, 403}:
            return (True, f"HTTP {exc.code} (protected)")
        return (False, f"HTTP {exc.code}")
    except Exception as exc:
        return (False, str(exc))

def main():
    if not MANIFEST_PATH.exists():
        print(f"Error: Manifest not found at {MANIFEST_PATH}")
        sys.exit(1)

    with open(MANIFEST_PATH, "r", encoding="utf-8") as f:
        manifest = json.load(f)

    domains = manifest.get("domains", [])
    print(f"--- HEIWA DOMAIN PROBE ---")
    print(f"Checking {len(domains)} domains from manifest...\n")

    all_ready = True

    for d in domains:
        host = d.get("host")
        health_path = d.get("health_path", "/")
        print(f"🌍 {host} ({d.get('purpose', 'Unknown')})")

        dns_ok = resolve_dns(host)
        if not dns_ok:
            print(f"   [FAIL] DNS: Could not resolve")
            all_ready = False
            continue
        
        print(f"   [OK]   DNS: Resolved")
        
        url = f"https://{host}{health_path}"
        http_ok, status = http_health(url)
        
        if http_ok:
            print(f"   [OK]   HTTP: {status}")
        else:
            print(f"   [FAIL] HTTP: {status}")
            all_ready = False
            
    print("\n--- SUMMARY ---")
    if all_ready:
        print("✅ All domains resolved and responding successfully.")
    else:
        print("⚠️ Some domains are pending cutover or failing.")

if __name__ == "__main__":
    main()
