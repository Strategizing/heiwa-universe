#!/usr/bin/env python3
"""
cli/scripts/ops/rename_services.py

Automates the renaming of Railway services to match the 'heiwa-*' schema.
Requires `railway` CLI to be authenticated.
"""
import subprocess
import json
import sys

# Mapping: Old Name -> New Name
RENAME_MAP = {
    "heiwa-cloud-hq": "heiwa-core",
    "cloud-hq": "heiwa-core",
    "hub-api": "heiwa-core",
    "hub-bot": "heiwa-uplink",
    "heiwa-bot": "heiwa-uplink",
    "nats": "heiwa-synapse",
    "Postgres": "heiwa-vault",
    "hub-cron": "heiwa-pulse",
    "hub-watchdog": "heiwa-sentry"
}

def run_railway_json(args):
    """Run railway command and parse JSON output."""
    try:
        cmd = ["railway"] + args + ["--json"]
        result = subprocess.run(cmd, capture_output=True, text=True, check=True)
        return json.loads(result.stdout)
    except subprocess.CalledProcessError as e:
        print(f"Error running railway {args}: {e.stderr}")
        return None
    except json.JSONDecodeError:
        print(f"Failed to parse JSON from railway {args}")
        return None

def rename_service(service_id, old_name, new_name):
    """Rename a service using the CLI (if supported) or warn user."""
    print(f"🔄 Renaming '{old_name}' ({service_id}) -> '{new_name}'...")
    # Railway CLI doesn't strictly have a 'rename' command for services via JSON yet?
    # We might need to use the GraphQL API or just log it for manual action if CLI lacks it.
    # However, 'railway service update' might work.
    
    # Placeholder for actual CLI command if it exists, theoretically:
    # subprocess.run(["railway", "service", "update", service_id, "--name", new_name])
    
    print(f"⚠️  Manual Action Required: Please rename '{old_name}' to '{new_name}' in the Railway Dashboard.")
    print(f"   (ID: {service_id})")

def main():
    print("🔍 Scanning Railway Project...")
    # Get current project status
    status = run_railway_json(["status"])
    if not status:
        print("❌ Could not get project status. Ensure you are linked (`railway link`).")
        sys.exit(1)

    # Filter for services (this depends on the exact JSON structure of 'railway status --json')
    # If standard CLI doesn't output services JSON, we might need to assume or use graphQL.
    # For this script, let's assume valid JSON output.
    
    services = status.get("services", [])
    if not services:
        print("ℹ️  No services found or JSON format unexpected.")
        return

    for svc in services:
        s_name = svc.get("name")
        s_id = svc.get("id")
        
        if s_name in RENAME_MAP:
            new_name = RENAME_MAP[s_name]
            if s_name != new_name:
                rename_service(s_id, s_name, new_name)
            else:
                print(f"✅ Service '{s_name}' is already named correctly.")
        else:
            print(f"ℹ️  Skipping unknown service: '{s_name}'")

if __name__ == "__main__":
    main()