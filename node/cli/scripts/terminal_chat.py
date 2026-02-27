import asyncio
import os
import json
import time
import sys
import uuid
import httpx
import psutil
from datetime import datetime
from nats.aio.client import Client as NATS

# Load environment variables from heiwa monorepo root
def load_env():
    # Try multiple possible paths for .env
    paths = [
        os.path.abspath(os.path.join(os.path.dirname(__file__), "../../../.env")),
        os.path.expanduser("~/heiwa/.env"),
        ".env"
    ]
    for path in paths:
        if os.path.exists(path):
            with open(path) as f:
                for line in f:
                    if line.strip() and not line.startswith('#'):
                        try:
                            key, val = line.strip().split('=', 1)
                            os.environ[key] = val.strip('"').strip("'")
                        except ValueError:
                            pass
            return True
    return False

load_env()

NATS_URL = os.getenv("NATS_URL", "nats://localhost:4222")
GEMINI_API_KEY = os.getenv("GEMINI_API_KEY", "")
OLLAMA_URL = os.getenv("OLLAMA_URL", "http://localhost:11434")

class TerminalChat:
    def __init__(self, username, node_id):
        self.username = username
        self.node_id = node_id
        self.nc = NATS()
        self.session_id = f"session-{uuid.uuid4().hex[:8]}"
        self.history = []
        self.start_time = time.time()

    async def connect(self):
        try:
            # We use a short timeout to prevent hanging if Railway is unreachable
            await self.nc.connect(NATS_URL, connect_timeout=2)
            return True
        except Exception:
            return False

    async def get_response(self, prompt):
        # Try local Ollama first (Tier 1)
        try:
            async with httpx.AsyncClient(timeout=30.0) as client:
                resp = await client.post(
                    f"{OLLAMA_URL}/api/generate",
                    json={
                        "model": "qwen2.5-coder:7b",
                        "prompt": f"You are Heiwa Node Assistant. User is {self.username}. Be concise.\n\nUser: {prompt}",
                        "stream": False
                    }
                )
                if resp.status_code == 200:
                    return resp.json().get("response", ""), "Ollama (Local)"
        except Exception:
            pass

        # Fallback to Gemini (Tier 2)
        if GEMINI_API_KEY:
            try:
                url = f"https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent?key={GEMINI_API_KEY}"
                payload = {
                    "contents": [{"parts": [{"text": f"User is {self.username}. Be concise.\n\nUser: {prompt}"}]}]
                }
                async with httpx.AsyncClient(timeout=30.0) as client:
                    resp = await client.post(url, json=payload)
                    data = resp.json()
                    if "candidates" in data:
                        return data["candidates"][0]["content"]["parts"][0]["text"], "Gemini (Cloud)"
            except Exception as e:
                return f"Error connecting to AI: {e}", "Error"

        return "No AI providers available. Check your NATS/API config.", "None"

    async def send_summary(self, delete=False):
        if delete:
            print("🗑️ Session summary deleted.")
            return

        summary_text = f"--- HEIWA SESSION SUMMARY ---\nID: {self.session_id}\nUser: {self.username}\nNode: {self.node_id}\nDuration: {int(time.time() - self.start_time)}s\nMessages: {len(self.history)}\n\n"
        for h in self.history:
            summary_text += f"{h['role']}: {h['content'][:150]}...\n"

        if self.nc.is_connected:
            payload = {
                "task_id": self.session_id,
                "agent": "TerminalChat",
                "content": summary_text,
                "result_type": "text",
                "status": "PASS",
                "timestamp": time.time()
            }
            # Publish to the global log subject so Railway's Messenger picks it up
            await self.nc.publish("heiwa.log.info", json.dumps(payload).encode())
            print(f"✅ Summary pushed to Cloud Swarm (#moltbook-logs).")
        else:
            print("⚠️ Cloud Swarm offline. Summary saved locally to ~/.heiwa/context/")
            log_path = os.path.expanduser(f"~/.heiwa/context/{self.session_id}.txt")
            with open(log_path, "w") as f:
                f.write(summary_text)

    async def run(self):
        print(f"\n🦁 HEIWA TERMINAL: {self.username} @ {self.node_id}")
        is_connected = await self.connect()
        print(f"📡 Swarm: {'CONNECTED' if is_connected else 'OFFLINE (Standalone)'}")
        
        ram = psutil.virtual_memory().percent
        print(f"💻 Health: RAM {ram}% | vCPU Active")
        print("-" * 50)
        
        while True:
            try:
                user_input = input(f"[{self.username}] > ").strip()
                if not user_input: continue
                if user_input.lower() in ["exit", "quit", "logout"]:
                    break
                
                self.history.append({"role": "user", "content": user_input})
                
                print("🧠 thinking...", end="\r")
                response, provider = await self.get_response(user_input)
                print(f"[{provider}] {response}")
                
                self.history.append({"role": "assistant", "content": response})
                
            except KeyboardInterrupt:
                break

        print("\n" + "-" * 50)
        save_req = input("Push session to Cloud Swarm? (Y/n): ").lower() != 'n'
        await self.send_summary(delete=not save_req)
        
        if self.nc.is_connected:
            await self.nc.close()
        print("🔒 Node detached. Session closed.")

if __name__ == "__main__":
    username = sys.argv[1] if len(sys.argv) > 1 else "devon"
    node_id = sys.argv[2] if len(sys.argv) > 2 else "terminal"
    chat = TerminalChat(username, node_id)
    asyncio.run(chat.run())
