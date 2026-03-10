import sys
from typing import Any

import requests

try:
    from heiwa_sdk.heiwa_net import HeiwaNetProxy
    _NET_PROXY = HeiwaNetProxy(origin_surface="runtime", agent_id="sanity-remote")
except ImportError:
    _NET_PROXY = None


def _safe_json(value: requests.Response) -> Any:
    try:
        return value.json()
    except Exception:
        return value.text


def run_remote_check(base_url: str) -> bool:
    base_url = base_url.rstrip("/")
    print(f"--- 📡 SATELLITE UPLINK: {base_url} ---")

    # 1. Health Check
    try:
        print("...pinging Health Check...")
        if _NET_PROXY:
            r = _NET_PROXY.get(f"{base_url}/", purpose="remote health check", purpose_class="health_check", timeout=15)
        else:
            r = requests.get(f"{base_url}/", timeout=15)
        r.raise_for_status()
        print(f"✅ HEALTH: {_safe_json(r)}")
    except Exception as e:
        print(f"❌ HEALTH FAILED: {e}")
        return False

    # 2. Product Flow
    try:
        payload = {
            "name": "Cloud Widget",
            "description": "Minted in production",
            "price": 99.99,
            "in_stock": True,
        }
        print("...testing Remote CREATE...")
        if _NET_PROXY:
            r = _NET_PROXY.post(f"{base_url}/api/v1/products", purpose="remote sanity create", purpose_class="api_data_write", json=payload, timeout=30)
        else:
            r = requests.post(f"{base_url}/api/v1/products", json=payload, timeout=30)
        r.raise_for_status()
        prod = r.json()
        print(f"✅ CREATE SUCCESS: ID={prod.get('id')}")

        print("...testing Remote READ...")
        if _NET_PROXY:
            r = _NET_PROXY.get(f"{base_url}/api/v1/products", purpose="remote sanity read", purpose_class="api_data_read", timeout=30)
        else:
            r = requests.get(f"{base_url}/api/v1/products", timeout=30)
        r.raise_for_status()
        products = r.json()
        found = any(p.get("id") == prod.get("id") for p in products)
        if found:
            print("✅ READ SUCCESS: Product confirmed in live DB.")
        else:
            print("❌ READ FAILED: Product not found.")
            return False
    except Exception as e:
        print(f"❌ PRODUCT FLOW FAILED: {e}")
        return False

    print("--- 🟢 REMOTE STATUS: OPERATIONAL ---")
    return True


def main() -> int:
    if len(sys.argv) < 2:
        print("❌ Usage: python verify_remote.py <URL>")
        return 1
    return 0 if run_remote_check(sys.argv[1]) else 1


if __name__ == "__main__":
    raise SystemExit(main())