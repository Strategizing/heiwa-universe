#!/usr/bin/env python3
"""
Verify Deployment Script
Wraps libs.heiwa_sdk.sanity.remote to perform a pulse check on the Cloud HQ.
"""
import sys
import os
from pathlib import Path

# Ensure runtime libs can be imported from the monorepo layout.
MONOREPO_ROOT = Path(__file__).resolve().parents[2]
RUNTIME_ROOT = MONOREPO_ROOT / "runtime"
if str(RUNTIME_ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))
    sys.path.insert(0, str(ROOT / "packages"))
    sys.path.insert(0, str(ROOT / "apps"))


def _ensure_requests_runtime() -> None:
    try:
        import requests  # noqa: F401
    except ModuleNotFoundError:
        venv_python = MONOREPO_ROOT / ".venv/bin/python"
        if venv_python.exists() and Path(sys.executable) != venv_python:
            os.execv(str(venv_python), [str(venv_python), __file__, *sys.argv[1:]])
        raise


_ensure_requests_runtime()
from heiwa_sdk.sanity.remote import run_remote_check

def main():
    print("--- 📡 HEIWA DEPLOYMENT VERIFICATION ---")
    
    # Try to find URL from args or env
    url = None
    if len(sys.argv) > 1:
        url = sys.argv[1]
    else:
        url = os.getenv("RAILWAY_PUBLIC_DOMAIN")
        if url and not url.startswith("http"):
            url = f"https://{url}"

    if not url:
        print("⚠️  No URL provided.")
        print("Usage: python cli/scripts/verify_deployment.py <https://your-app.railway.app>")
        sys.exit(1)

    print(f"Target: {url}")
    
    ok = run_remote_check(url)
    if ok:
        print("\n✅ DEPLOYMENT VERIFICATION PASSED")
        return

    print("\n❌ DEPLOYMENT VERIFICATION FAILED")
    sys.exit(1)

if __name__ == "__main__":
    main()