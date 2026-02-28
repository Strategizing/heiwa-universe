import os
import requests
import asyncio
import subprocess
from pathlib import Path

VAULT_PATH = Path.home() / ".heiwa" / "vault.env"

def load_all_env_vars():
    envs = {}
    paths = [Path(".env"), Path(".env.worker.local"), Path(".env.workstation"), VAULT_PATH]
    for p in paths:
        if p.exists():
            with open(p, "r") as f:
                for line in f:
                    line = line.strip()
                    if line and not line.startswith("#") and "=" in line:
                        k, v = line.split("=", 1)
                        envs[k.strip()] = v.strip().strip('"').strip("'")
    return envs

async def test_google(key):
    url = f"https://generativelanguage.googleapis.com/v1beta/models?key={key}"
    try:
        resp = requests.get(url, timeout=5)
        return resp.status_code == 200, f"HTTP {resp.status_code}"
    except Exception as e:
        return False, str(e)

async def test_groq(key):
    url = "https://api.groq.com/openai/v1/models"
    headers = {"Authorization": f"Bearer {key}"}
    try:
        resp = requests.get(url, headers=headers, timeout=5)
        return resp.status_code == 200, f"HTTP {resp.status_code}"
    except Exception as e:
        return False, str(e)

async def test_openrouter(key):
    url = "https://openrouter.ai/api/v1/models"
    headers = {"Authorization": f"Bearer {key}"}
    try:
        resp = requests.get(url, headers=headers, timeout=5)
        return resp.status_code == 200, f"HTTP {resp.status_code}"
    except Exception as e:
        return False, str(e)

async def test_railway(token):
    # Railway CLI check
    try:
        env = os.environ.copy()
        env["RAILWAY_TOKEN"] = token
        result = subprocess.run(["railway", "status"], env=env, capture_output=True, text=True, timeout=10)
        return result.returncode == 0, result.stdout.strip() or result.stderr.strip()
    except Exception as e:
        return False, str(e)

async def main():
    print("🕵️  Heiwa Enterprise Secret Audit...")
    envs = load_all_env_vars()
    
    results = {}
    
    if "GOOGLE_API_KEY" in envs:
        ok, msg = await test_google(envs["GOOGLE_API_KEY"])
        results["Google (Gemini)"] = (ok, msg)
    
    if "GROQ_API_KEY" in envs:
        ok, msg = await test_groq(envs["GROQ_API_KEY"])
        results["Groq"] = (ok, msg)
        
    if "OPENROUTER_API_KEY" in envs:
        ok, msg = await test_openrouter(envs["OPENROUTER_API_KEY"])
        results["OpenRouter"] = (ok, msg)
        
    if "RAILWAY_TOKEN" in envs:
        ok, msg = await test_railway(envs["RAILWAY_TOKEN"])
        results["Railway"] = (ok, msg)

    print("\n--- AUDIT RESULTS ---")
    for svc, (ok, msg) in results.items():
        status = "✅ VALID" if ok else "❌ FAILED"
        print(f"{svc:<20}: {status} ({msg[:50]})")

if __name__ == "__main__":
    asyncio.run(main())
