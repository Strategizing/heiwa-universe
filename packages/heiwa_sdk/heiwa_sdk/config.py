import os
import re
import sys
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
    """Recursively search for the monorepo root. Honors HEIWA_ROOT env var."""
    explicit = os.getenv("HEIWA_ROOT")
    if explicit:
        p = Path(explicit).resolve()
        if (p / "apps").exists() and (p / "packages").exists():
            return p
    current = Path(__file__).resolve()
    for _ in range(5):
        if (current.parent / "apps").exists() and (current.parent / "packages").exists():
            return current.parent
        current = current.parent
    raise RuntimeError(
        "Could not discover Heiwa monorepo root (looked for apps/ + packages/ "
        "up to 5 levels from sdk). Set HEIWA_ROOT or run from inside the repo."
    )

MONOREPO_ROOT = _find_monorepo_root()


def _unique_strings(values: list[str]) -> list[str]:
    seen: set[str] = set()
    ordered: list[str] = []
    for raw in values:
        value = str(raw or "").strip().rstrip("/")
        if not value or value in seen:
            continue
        seen.add(value)
        ordered.append(value)
    return ordered


def _profile_hub_fallbacks() -> list[str]:
    profile_path = MONOREPO_ROOT / "config" / "swarm" / "profiles" / "heiwa-one-system.yaml"
    if not profile_path.exists():
        return []
    try:
        text = profile_path.read_text(encoding="utf-8")
    except Exception:
        return []
    matches = re.findall(r"https://[A-Za-z0-9._-]+", text)
    return [url for url in matches if "up.railway.app" in url or url.endswith("api.heiwa.ltd")]


def hub_url_candidates() -> list[str]:
    direct_env = [
        os.getenv("HEIWA_HUB_URL", ""),
        os.getenv("HEIWA_HUB_BASE_URL", ""),
        os.getenv("HEIWA_HUB_FALLBACK_URL", ""),
    ]
    profile_urls = _profile_hub_fallbacks()
    defaults = ["https://api.heiwa.ltd", "https://heiwa-cloud-hq-brain.up.railway.app"]
    return _unique_strings(direct_env + defaults + profile_urls)


def _bool_env(key: str) -> bool:
    return str(os.getenv(key, "")).strip().lower() in {"1", "true", "yes", "on"}


def load_swarm_env():
    """Enterprise-grade environment loader. Priority: Vault > Local Worker > Standard Env."""
    vault_path = Path.home() / ".heiwa" / "vault.env"
    
    # 1. Standard .env (Base)
    env_path = MONOREPO_ROOT / ".env"
    if env_path.exists():
        load_dotenv(env_path, override=True)
    
    # 2. Worker Local (Specific Node overrides)
    worker_env = MONOREPO_ROOT / ".env.worker.local"
    if worker_env.exists():
        load_dotenv(worker_env, override=True)
    
    # 3. Vault (Highest priority secrets)
    if vault_path.exists():
        load_dotenv(vault_path, override=True)

class Settings:
    # --- VERSIONING ---
    @property
    def HEIWA_VERSION(self): return "v1.67"

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
    def HEIWA_ENABLE_BRIDGE(self): return get_env("HEIWA_ENABLE_BRIDGE", default="false", required=False).lower() == "true"
    
    @property
    def HUB_BASE_URL(self):
        return hub_url_candidates()[0]

    @property
    def HUB_FALLBACK_URLS(self):
        candidates = hub_url_candidates()
        return candidates[1:] if len(candidates) > 1 else []
    
    @property
    def PORT(self): return int(get_env("PORT", default=8000, required=False))
    
    # --- DATABASE ---
    @property
    def DATABASE_URL(self): return get_env("DATABASE_URL", required=False)
    
    @property
    def DATABASE_PATH(self): return get_env("DATABASE_PATH", default="./hub.db", required=False)

    @property
    def HEIWA_STATE_BACKEND(self):
        default = "spacetimedb" if self.IS_PROD else "compatibility_sqlite"
        return get_env("HEIWA_STATE_BACKEND", default=default, required=False)

    @property
    def STDB_IDENTITY(self):
        return get_env("STDB_IDENTITY", required=False)

    @property
    def STDB_SERVER(self):
        return get_env("STDB_SERVER", default="maincloud", required=False)

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

    @property
    def PHASE2_WRITE_ENABLED(self):
        return get_env("PHASE2_WRITE_ENABLED", default="true", required=False).lower() == "true"

    @property
    def PHASE2_CLAIM_ENABLED(self):
        return get_env("PHASE2_CLAIM_ENABLED", default="true", required=False).lower() == "true"

    @property
    def PHASE2_ROUTER_ENABLED(self):
        return get_env("PHASE2_ROUTER_ENABLED", default="true", required=False).lower() == "true"
    
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
        if self.HEIWA_STATE_BACKEND != "compatibility_postgres":
            return False
        db_url = self.DATABASE_URL
        if not db_url or not db_url.startswith(("postgres://", "postgresql://")):
            return False
        if any(token in db_url for token in ("${{", "}}", "{", "}")):
            return False
        parsed = urlparse(db_url)
        return bool(parsed.hostname)

settings = Settings()
