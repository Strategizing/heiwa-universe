#!/usr/bin/env python3
"""
Syncs the authoritative domain manifest to the public web assets directory.
Ensures clients/clients/web always reflects infrastructure/truth.
"""

import json
import sys
from pathlib import Path
import shutil

ROOT = Path(__file__).resolve().parents[4]
SOURCE_MANIFEST = ROOT / "infrastructure" / "domains" / "heiwa-ltd.bootstrap.json"
DEST_MANIFEST = ROOT / "clients" / "clients" / "web" / "assets" / "domains.bootstrap.json"

def main():
    print("--- HEIWA MANIFEST SYNC ---")
    if not SOURCE_MANIFEST.exists():
        print(f"❌ Source manifest not found: {SOURCE_MANIFEST}")
        sys.exit(1)

    # Validate JSON syntax
    try:
        with open(SOURCE_MANIFEST, "r", encoding="utf-8") as f:
            data = json.load(f)
    except json.JSONDecodeError as e:
        print(f"❌ Invalid JSON in source manifest: {e}")
        sys.exit(1)
        
    # Copy file securely
    DEST_MANIFEST.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(SOURCE_MANIFEST, DEST_MANIFEST)
    print(f"✅ Synced {SOURCE_MANIFEST.relative_to(ROOT)} -> {DEST_MANIFEST.relative_to(ROOT)}")
    sys.exit(0)

if __name__ == "__main__":
    main()
