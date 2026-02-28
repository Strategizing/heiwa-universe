import os
import sys
from urllib.parse import urlparse
from pathlib import Path
from dotenv import load_dotenv

def get_env(key, default=None, required=True):
    """Strict environment variable fetcher. Crashes on missing required keys."""
    val = os.getenv(key, default)
    if required and not val:
        # Only crash in production or if explicitly requested
        if os.getenv("RAILWAY_ENVIRONMENT_NAME") == "production":
            print(f"\n[FATAL] 🚨 SDK Configuration Error")
            print(f"[FATAL] Missing required environment variable: {key}")
            sys.exit(1)
    return val

def load_swarm_env():
    """Enterprise-grade environment loader. Priority: Vault > Local Worker > Standard Env."""
    # Find monorepo root dynamically
    current = Path(__file__).resolve()
    monorepo_root = None
    for _ in range(5):
        if (current.parent / "apps").exists() and (current.parent / "packages").exists():
            monorepo_root = current.parent
            break
        current = current.parent
    
    if not monorepo_root:
        monorepo_root = Path("/Users/dmcgregsauce/heiwa")
        
    vault_path = Path.home() / ".heiwa" / "vault.env"
    
    # 1. Standard .env (Base)
    load_dotenv(monorepo_root / ".env")
    
    # 2. Worker Local (Specific Node overrides)
    load_dotenv(monorepo_root / ".env.worker.local", override=True)
    
    # 3. Vault (Highest priority secrets)
    if vault_path.exists():
        load_dotenv(vault_path, override=True)

class Settings:
    # --- IDENTITY & SOVEREIGNTY ---
    HEIWA_AUTH_TOKEN = get_env("HEIWA_AUTH_TOKEN", required=False)
    OWNER_ID = get_env("OWNER_ID", required=False)
    
    # --- HUB & MESH ---
    NATS_URL = get_env("NATS_URL", default="nats://localhost:4222", required=False)
    HUB_BASE_URL = get_env("HEIWA_HUB_BASE_URL", required=False)
    PORT = int(get_env("PORT", default=8000, required=False))
    
    # --- DATABASE ---
    DATABASE_URL = get_env("DATABASE_URL", required=False)
    DATABASE_PATH = get_env("DATABASE_PATH", default="./hub.db", required=False)

    # --- DISCORD ---
    DISCORD_BOT_TOKEN = get_env("DISCORD_BOT_TOKEN", required=False)
    DISCORD_APPLICATION_ID = get_env("DISCORD_APPLICATION_ID", required=False)
    DISCORD_GUILD_ID = get_env("DISCORD_GUILD_ID", default="0", required=False)
    DISCORD_WEBHOOK_URL = get_env("DISCORD_WEBHOOK_URL", required=False)

    # --- LLM & WORKER ---
    HEIWA_LLM_MODE = get_env("HEIWA_LLM_MODE", default="local_only", required=False)
    HEIWA_WORKER_WARM_TTL_SEC = int(get_env("HEIWA_WORKER_WARM_TTL_SEC", default="600", required=False) or "600")
    HEIWA_EXECUTOR_CONCURRENCY = int(get_env("HEIWA_EXECUTOR_CONCURRENCY", default="4", required=False) or "4")
    
    # --- INFRA ---
    RAILWAY_ENVIRONMENT_NAME = get_env("RAILWAY_ENVIRONMENT_NAME", default="development", required=False)
    IS_PROD = (RAILWAY_ENVIRONMENT_NAME == "production")

    @property
    def use_postgres(self) -> bool:
        """Check if we should use Postgres."""
        db_url = self.DATABASE_URL
        if not db_url or not db_url.startswith(("postgres://", "postgresql://")):
            return False
        if any(token in db_url for token in ("${{", "}}", "{", "}")):
            return False
        parsed = urlparse(db_url)
        return bool(parsed.hostname)

settings = Settings()
