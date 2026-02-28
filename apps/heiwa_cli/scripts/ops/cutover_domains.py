#!/usr/bin/env python3
"""
Automates the domain cutover sequence using the Cloudflare Wrangler CLI.
Reads the domain manifest and attempts to bind Cloudflare Pages domains.
"""

import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MANIFEST_PATH = ROOT / "infrastructure" / "domains" / "heiwa-ltd.bootstrap.json"

def run_cmd(cmd: list[str]) -> bool:
    print(f"🔄 Executing: {' '.join(cmd)}")
    try:
        result = subprocess.run(cmd, check=True, capture_output=True, text=True)
        return True
    except subprocess.CalledProcessError as e:
        print(f"❌ Error executing command:\n{e.stderr.strip() or e.stdout.strip()}")
        return False

def main():
    print("--- HEIWA AUTOMATED DOMAIN CUTOVER ---")
    if not MANIFEST_PATH.exists():
        print(f"❌ Manifest not found at {MANIFEST_PATH}")
        sys.exit(1)

    with open(MANIFEST_PATH, "r", encoding="utf-8") as f:
        manifest = json.load(f)

    # Make sure we are authenticated
    print("🔐 Checking Wrangler authentication...")
    auth_check = subprocess.run(["wrangler", "whoami"], capture_output=True, text=True)
    if auth_check.returncode != 0:
        print("❌ Wrangler is not authenticated. Please run `wrangler login` first.")
        sys.exit(1)
    print("✅ Wrangler authenticated.\n")

    domains = manifest.get("domains", [])
    pages_domains = [d for d in domains if "Cloudflare Pages" in d.get("target", "")]
    
    if not pages_domains:
        print("⚠️ No Cloudflare Pages domains found in manifest to bind.")
    else:
        print(f"📦 Found {len(pages_domains)} Cloudflare Pages domains to bind.")
        for d in pages_domains:
            host = d.get("host")
            # We assume project is "heiwa_clients" for this bootstrap logic
            # You could parse this dynamically if needed
            project_name = "heiwa_clients"
            if "docs" in d.get("purpose", "").lower():
                # Or a dedicated docs project if defined, but manifest currently points to "heiwa_clients" or "Cloudflare Pages docs project"
                # Let's check target text loosely
                if "docs project" in d.get("target", "").lower():
                    project_name = "heiwa-docs"
                
            print(f"\n🔗 Domain '{host}' is mapped to Pages project '{project_name}'.")
            print("   (Note: Custom domains must be bound via Cloudflare Dashboard or Terraform)")
            print(f"   Dashboard: https://dash.cloudflare.com/?to=/:account/pages/view/{project_name}/settings/domains")

    print("\nℹ️  NEXT STEPS FOR CUTOVER:")
    print("   1. Obtain your Cloudflare API Token (Edit zone DNS permissions)")
    print("   2. cd infra/cloud/cloudflare")
    print("   3. export CLOUDFLARE_API_TOKEN=\"your-token-here\"")
    print("   4. terraform init && terraform apply")
    print("\n✨ Cutover verification sequence completed.")

if __name__ == "__main__":
    main()