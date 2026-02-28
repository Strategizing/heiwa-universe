import os
import sys
from urllib.parse import urlparse

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

class Settings:
    # Database: prefer DATABASE_URL (Postgres)
    DATABASE_URL = get_env("DATABASE_URL")
    DATABASE_PATH = get_env("DATABASE_PATH", default="./hub.db", required=False)

    # Auth
    AUTH_TOKEN = get_env("HEIWA_AUTH_TOKEN")
    HUB_BASE_URL = get_env("HEIWA_HUB_BASE_URL", required=False)
    PORT = int(get_env("PORT", default=8000, required=False))

    # Discord Notification Settings
    DISCORD_WEBHOOK_URL = get_env("DISCORD_WEBHOOK_URL", required=False)

    # Tick Service Mode: 'production' or 'simulation'
    TICK_MODE = get_env("TICK_MODE", default="production", required=False)

    # Environment Name (for display/logging)
    HEIWA_ENV = get_env("RAILWAY_ENVIRONMENT_NAME", default="development", required=False)

    # Node Liveness Thresholds
    NODE_SILENT_AFTER_MINUTES = int(get_env("NODE_SILENT_AFTER_MINUTES", default="10", required=False))
    NODE_OFFLINE_AFTER_MINUTES = int(get_env("NODE_OFFLINE_AFTER_MINUTES", default="60", required=False))

    # Phase 2 Feature Flags
    PHASE2_WRITE_ENABLED = get_env("PHASE2_WRITE_ENABLED", default="0", required=False) == "1"
    PHASE2_DISCORD_BUTTONS_ENABLED = get_env("PHASE2_DISCORD_BUTTONS_ENABLED", default="0", required=False) == "1"
    PHASE2_ROUTER_ENABLED = get_env("PHASE2_ROUTER_ENABLED", default="0", required=False) == "1"
    PHASE2_CLAIM_ENABLED = get_env("PHASE2_CLAIM_ENABLED", default="0", required=False) == "1"

    @property
    def use_postgres(self) -> bool:
        """Check if we should use Postgres."""
        db_url = self.DATABASE_URL
        if not db_url:
            return False
        if not db_url.startswith(("postgres://", "postgresql://")):
            return False
        # Railway template placeholders should not force local Postgres mode.
        if any(token in db_url for token in ("${{", "}}", "{", "}")):
            return False
        parsed = urlparse(db_url)
        return bool(parsed.hostname)


settings = Settings()
