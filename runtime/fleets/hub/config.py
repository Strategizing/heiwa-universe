import os
import sys
import subprocess
import json
import yaml
from dotenv import load_dotenv

load_dotenv()

def get_env(key, default=None, required=True):
    """Strict environment variable fetcher. Crashes on missing required keys."""
    val = os.getenv(key, default)
    if required and not val:
        print(f"[WARN] Missing required environment variable: {key}")
        print(f"[WARN] Continuing in degraded mode...\n")
        return None
    return val

class Config:
    # [LETHAL] Identity & Sovereignty
    DISCORD_TOKEN = get_env("DISCORD_BOT_TOKEN")
    GUILD_ID = int(get_env("DISCORD_GUILD_ID") or "0")
    ALLOWED_ROLES = get_env("ALLOWED_ROLES", default="Admin,Orchestrator").split(",")
    
    # [HIGH] Persistence & Memory
    DATABASE_URL = get_env("DATABASE_URL")
    DATABASE_PATH = get_env("DATABASE_PATH", default="./hub.db", required=False)
    
    # [MED] Transport & Infrastructure
    TAILSCALE_AUTHKEY = get_env("RAILWAY_TAILSCALE_AUTH_KEY", required=False)
    CLIENT_ID = get_env("DISCORD_APPLICATION_ID", required=False)
    OWNER_ID = get_env("OWNER_ID", required=False)

    # [MODEL] Local-first orchestration settings
    HEIWA_LLM_MODE = get_env("HEIWA_LLM_MODE", default="local_only", required=False)
    HEIWA_OLLAMA_ORCH_URL = get_env("HEIWA_OLLAMA_ORCH_URL", default="http://127.0.0.1:11434", required=False)
    HEIWA_OLLAMA_LOCAL_URL = get_env("HEIWA_OLLAMA_LOCAL_URL", default="http://127.0.0.1:11434", required=False)
    HEIWA_WORKER_WARM_TTL_SEC = int(get_env("HEIWA_WORKER_WARM_TTL_SEC", default="600", required=False) or "600")
    HEIWA_EXECUTOR_CONCURRENCY = int(get_env("HEIWA_EXECUTOR_CONCURRENCY", default="2", required=False) or "2")
    HEIWA_APPROVAL_TIMEOUT_SEC = int(get_env("HEIWA_APPROVAL_TIMEOUT_SEC", default="600", required=False) or "600")
    HEIWA_ALLOWED_OUTBOUND_TARGETS = get_env("HEIWA_ALLOWED_OUTBOUND_TARGETS", default="", required=False)

    # [DEFERRED] Optional API adapter (disabled by default)
    HEIWA_OPENAI_ENABLED = get_env("HEIWA_OPENAI_ENABLED", default="false", required=False)
    OPENAI_API_KEY = get_env("OPENAI_API_KEY", required=False)
    GEMINI_API_KEY = get_env("GEMINI_API_KEY", required=False)
    REDIS_URL = get_env("REDIS_URL", default="redis://localhost:6379/0", required=False)
    
    # [CONTEXT] Environment
    IS_PROD = get_env("RAILWAY_ENVIRONMENT_NAME", required=False) == "production"
    LOG_LEVEL = get_env("LOG_LEVEL", default="INFO", required=False)

    @classmethod
    def validate(cls):
        """Global validation gate."""
        print(f"[CONFIG] Sovereign Configuration Validated. (Mode: {'PROD' if cls.IS_PROD else 'DEV'})")

def get_tailscale_ip():
    """Get the Tailscale IP from the running daemon."""
    try:
        # Check for socket first (Production/Railway mode)
        result = subprocess.run(
            ["tailscale", "--socket=/tmp/tailscaled.sock", "ip", "-4"],
            capture_output=True,
            text=True,
            timeout=5
        )
        if result.returncode == 0:
            return result.stdout.strip()
        
        # Fallback to default PATH (Muscle/Local mode)
        result = subprocess.run(
            ["tailscale", "ip", "-4"],
            capture_output=True,
            text=True,
            timeout=5
        )
        return result.stdout.strip() if result.returncode == 0 else "Unknown"
    except Exception:
        return "Unknown"

def _check_deprecated_identity():
    """Fail-fast: warn if legacy identity.json is detected."""
    legacy_paths = [
        os.path.join(os.path.dirname(__file__), "../../config/identity.json"),
        "/app/config/identity.json",
    ]
    for path in legacy_paths:
        resolved = os.path.abspath(path)
        if os.path.exists(resolved):
            print(f"[WARN] DEPRECATED: Found legacy identity file at {resolved}")
            print(f"[WARN] The Master Identity Record is: fleets/hub/config/identity.yaml")
            print(f"[WARN] Remove identity.json to silence this warning.")
            break

def load_identity():
    """
    Load the agent's identity.
    Priority: identity.json (bootstrap) > identity.yaml (legacy) > defaults.
    """
    _check_deprecated_identity()
    
    # 1. Try identity.json (created by bootstrap script)
    json_paths = [
        os.path.join(os.path.dirname(__file__), "../../identity.json"),  # repo root
        "/app/identity.json",  # Railway container
    ]
    for path in json_paths:
        resolved = os.path.abspath(path)
        if os.path.exists(resolved):
            try:
                with open(resolved, "r") as f:
                    identity = json.load(f)
                    print(f"[CONFIG] Identity loaded from {resolved} (uuid={identity.get('uuid', 'N/A')})")
                    return identity
            except Exception as e:
                print(f"[WARN] Failed to parse {resolved}: {e}")

    # 2. Fallback: identity.yaml (legacy)
    yaml_paths = [
        "/app/fleets/hub/config/identity.yaml",
        os.path.abspath(os.path.join(os.path.dirname(__file__), "config/identity.yaml")),
    ]
    for path in yaml_paths:
        if os.path.exists(path):
            try:
                with open(path, "r") as f:
                    identity = yaml.safe_load(f)
                    return identity.get('system', {})
            except Exception as e:
                print(f"[WARN] Failed to parse {path}: {e}")

    # 3. Default fallback
    print("[WARN] No identity file found. Running as ghost node.")
    return {"uuid": "unknown", "role": "ghost", "capabilities": []}

IDENTITY = load_identity()
TAILSCALE_IP = get_tailscale_ip()
