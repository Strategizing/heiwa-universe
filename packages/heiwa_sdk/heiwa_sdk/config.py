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

def _find_monorepo_root() -> Path:
    """Recursively search for the monorepo root."""
    current = Path(__file__).resolve()
    for _ in range(5):
        if (current.parent / "apps").exists() and (current.parent / "packages").exists():
            return current.parent
        current = current.parent
    return Path("/Users/dmcgregsauce/heiwa")

MONOREPO_ROOT = _find_monorepo_root()

def load_swarm_env():
    """Enterprise-grade environment loader. Priority: Vault > Local Worker > Standard Env."""
    vault_path = Path.home() / ".heiwa" / "vault.env"
    
    # 1. Standard .env (Base)
    load_dotenv(MONOREPO_ROOT / ".env")
    
    # 2. Worker Local (Specific Node overrides)
    load_dotenv(MONOREPO_ROOT / ".env.worker.local", override=True)
    
    # 3. Vault (Highest priority secrets)
    if vault_path.exists():
        load_dotenv(vault_path, override=True)

class Settings:
    # --- IDENTITY & SOVEREIGNTY ---
    @property
    def MONOREPO_ROOT(self): return MONOREPO_ROOT
    
    @property
    def HEIWA_AUTH_TOKEN(self): return get_env("HEIWA_AUTH_TOKEN", required=False)
    
    @property
    def OWNER_ID(self): return get_env("OWNER_ID", required=False)
    
    @property
    def HEIWA_AUTH_MODE(self): return get_env("HEIWA_AUTH_MODE", default="payload_token", required=False)
    
    # --- HUB & MESH ---
    @property
    def NATS_URL(self): return get_env("NATS_URL", default="tls://heiwa-cloud-hq-brain.up.railway.app:443", required=False)
    
    @property
    def HEIWA_ENABLE_BRIDGE(self): return get_env("HEIWA_ENABLE_BRIDGE", default="false", required=False).lower() == "true"
    
    @property
    def HUB_BASE_URL(self): return get_env("HEIWA_HUB_BASE_URL", required=False)
    
    @property
    def PORT(self): return int(get_env("PORT", default=8000, required=False))
    
    # --- DATABASE ---
    @property
    def DATABASE_URL(self): return get_env("DATABASE_URL", required=False)
    
    @property
    def DATABASE_PATH(self): return get_env("DATABASE_PATH", default="./hub.db", required=False)

    # --- DISCORD ---
    @property
    def DISCORD_BOT_TOKEN(self): return get_env("DISCORD_BOT_TOKEN", required=False)
    
    @property
    def DISCORD_APPLICATION_ID(self): return get_env("DISCORD_APPLICATION_ID", required=False)
    
    @property
    def DISCORD_GUILD_ID(self): return get_env("DISCORD_GUILD_ID", default="0", required=False)
    
    @property
    def DISCORD_WEBHOOK_URL(self): return get_env("DISCORD_WEBHOOK_URL", required=False)

    # --- LLM & WORKER ---
    @property
    def HEIWA_LLM_MODE(self): return get_env("HEIWA_LLM_MODE", default="local_only", required=False)
    
    @property
    def HEIWA_WORKER_WARM_TTL_SEC(self): return int(get_env("HEIWA_WORKER_WARM_TTL_SEC", default="600", required=False) or "600")
    
    @property
    def HEIWA_EXECUTOR_CONCURRENCY(self): return int(get_env("HEIWA_EXECUTOR_CONCURRENCY", default="4", required=False) or "4")
    
    # --- INFRA ---
    @property
    def RAILWAY_ENVIRONMENT_NAME(self): return get_env("RAILWAY_ENVIRONMENT_NAME", default="development", required=False)
    
    @property
    def IS_PROD(self): return (self.RAILWAY_ENVIRONMENT_NAME == "production")
    
    # --- HARVESTED CONFIGS ---
    @property
    def AI_ROUTER_PATH(self): return MONOREPO_ROOT / "config/swarm/ai_router.json"
    
    @property
    def MESSAGING_CHANNELS_PATH(self): return MONOREPO_ROOT / "config/swarm/messaging_channels.json"
    
    @property
    def OPERATOR_PROFILE_PATH(self): return MONOREPO_ROOT / "config/swarm/operator_profile.md"

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
