#!/usr/bin/env python3
"""
Verify Deployment Script
Wraps libs.heiwa_sdk.sanity.remote to perform a pulse check on the Cloud HQ.
"""
import sys
import os
from pathlib import Path

# Ensure libs can be imported
sys.path.append(str(Path(__file__).parent.parent))
from libs.heiwa_sdk.sanity.remote import run_remote_check

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
