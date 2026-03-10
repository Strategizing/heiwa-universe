import json
import os
from pathlib import Path
import uuid

HEIWA_GLOBAL_DIR = Path(os.path.expanduser("~/.heiwa"))
IDENTITY_PATH = HEIWA_GLOBAL_DIR / "identity.json"

def bootstrap():
    HEIWA_GLOBAL_DIR.mkdir(parents=True, exist_ok=True)
    
    if IDENTITY_PATH.exists():
        print(f"✅ Identity found: {IDENTITY_PATH}")
        return

    print(f"⚠️  Identity missing at {IDENTITY_PATH}. Generating new node identity...")
    identity = {
        "uuid": str(uuid.uuid4()),
        "role": "heiwa-node",
        "capabilities": ["compute", "orchestration"]
    }

    with open(IDENTITY_PATH, "w") as f:
        json.dump(identity, f, indent=4)

    print(f"✅ Created new identity: {identity['uuid']}")

if __name__ == "__main__":
    bootstrap()