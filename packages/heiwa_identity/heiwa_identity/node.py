import json
import os
from pathlib import Path
from typing import Dict, Any

def get_monorepo_root() -> Path:
    current = Path(__file__).resolve()
    for _ in range(5):
        if (current.parent / "apps").exists() and (current.parent / "packages").exists():
            return current.parent
        current = current.parent
    return Path("/Users/dmcgregsauce/heiwa")

def load_node_identity() -> Dict[str, Any]:
    """
    Load the current node's identity from identity.json.
    Checks common locations (root, ~/.heiwa, /app).
    """
    root = get_monorepo_root()
    search_paths = [
        root / "identity.json",
        Path.home() / ".heiwa" / "identity.json",
        Path("/app/identity.json")
    ]
    
    for path in search_paths:
        if path.exists():
            try:
                with open(path, "r") as f:
                    return json.load(f)
            except: pass
            
    return {"uuid": "unknown", "name": "ghost-node", "role": "worker", "capabilities": []}

def get_tailscale_ip() -> str:
    """Get the current node's Tailscale IP."""
    import subprocess
    try:
        # Try local first
        result = subprocess.run(["tailscale", "ip", "-4"], capture_output=True, text=True, timeout=2)
        if result.returncode == 0:
            return result.stdout.strip()
        # Try Railway socket
        result = subprocess.run(["tailscale", "--socket=/tmp/tailscaled.sock", "ip", "-4"], capture_output=True, text=True, timeout=2)
        if result.returncode == 0:
            return result.stdout.strip()
    except: pass
    return "127.0.0.1"
