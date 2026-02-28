#!/usr/bin/env python3
import json
import os
from pathlib import Path

VAULT_PATH = Path.home() / ".heiwa" / "vault.env"
AUTH_PROFILES_PATH = Path.home() / ".openclaw" / "agents" / "main" / "agent" / "auth-profiles.json"

def load_vault():
    secrets = {}
    if not VAULT_PATH.exists():
        return secrets
    with open(VAULT_PATH, "r") as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            if "=" in line:
                key, val = line.split("=", 1)
                secrets[key.strip()] = val.strip().strip('"').strip("'")
    return secrets

def update_auth_profiles(secrets):
    if not AUTH_PROFILES_PATH.exists():
        print(f"❌ Auth profiles not found at {AUTH_PROFILES_PATH}")
        return

    with open(AUTH_PROFILES_PATH, "r") as f:
        data = json.load(f)

    profiles = data.get("profiles", {})
    
    mapping = {
        "GROQ_API_KEY": ("groq", "groq:default"),
        "CEREBRAS_API_KEY": ("cerebras", "cerebras:default"),
        "OPENROUTER_API_KEY": ("openrouter", "openrouter:default"),
        "MISTRAL_API_KEY": ("mistral", "mistral:default"),
        "GOOGLE_API_KEY": ("google", "google:default")
    }

    updated = False
    for env_key, (provider, profile_id) in mapping.items():
        if env_key in secrets:
            profiles[profile_id] = {
                "type": "api_key",
                "provider": provider,
                "key": secrets[env_key]
            }
            updated = True
            print(f"✅ Injected key for {provider}")

    if updated:
        data["profiles"] = profiles
        with open(AUTH_PROFILES_PATH, "w") as f:
            json.dump(data, f, indent=2)
        print("🚀 OpenClaw Auth Profiles synchronized.")

if __name__ == "__main__":
    secrets = load_vault()
    update_auth_profiles(secrets)
