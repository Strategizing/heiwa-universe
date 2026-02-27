import asyncio
import os
import json
import logging
import psutil
import httpx
from datetime import datetime, timezone
from nats.aio.client import Client as NATS
import uuid

# Load environment variables
env_path = os.path.join(os.path.dirname(__file__), '.env')
if os.path.exists(env_path):
    with open(env_path) as f:
        for line in f:
            if line.strip() and not line.startswith('#'):
                try:
                    key, val = line.strip().split('=', 1)
                    os.environ[key] = val.strip()
                except ValueError:
                    pass

logging.basicConfig(level=logging.INFO, format="%(asctime)s - %(name)s - %(levelname)s - %(message)s")
logger = logging.getLogger("MacbookNode")

NATS_URL = os.getenv("NATS_URL", "nats://localhost:4222")
OLLAMA_URL = os.getenv("OLLAMA_URL", "http://localhost:11434")
NODE_ID = os.getenv("NODE_ID", "Macbook-GPU-Node")
MODEL_PRIMARY = os.getenv("MODEL_PRIMARY", "deepseek-coder-v2:16b")
MODEL_SECONDARY = os.getenv("MODEL_SECONDARY", "qwen2.5-coder:7b")

class InferenceNode:
    def __init__(self):
        self.nc = NATS()
        self.total_tokens = 0
        self.running = False

    async def connect(self):
        try:
            await self.nc.connect(NATS_URL)
            logger.info(f"✅ Connected to NATS Swarm at {NATS_URL}")
            self.running = True
        except Exception as e:
            logger.error(f"❌ Failed to connect to NATS: {e}")
            return False
        return True

    async def _handle_inference_request(self, msg):
        try:
            data = json.loads(msg.data.decode())
            prompt = data.get("prompt", "")
            system = data.get("system", "")
            complexity = data.get("complexity", "low")
            reply_to = msg.reply

            if not prompt or not reply_to:
                return

            model = MODEL_PRIMARY if complexity == "high" else MODEL_SECONDARY
            logger.info(f"🧠 Received {complexity} inference request. Routing to {model}...")

            full_prompt = f"System: {system}\n\nUser: {prompt}" if system else prompt
            payload = {
                "model": model,
                "prompt": full_prompt,
                "stream": False,
                "options": {"temperature": 0.2, "num_ctx": 8192}
            }

            # Check Macbook RAM before fulfilling
            ram = psutil.virtual_memory().percent
            if ram > 90.0:
                logger.warning(f"⚠️ System RAM at {ram}%. Rejecting request to protect Macbook.")
                await self.nc.publish(reply_to, json.dumps({"error": "Macbook overloaded (RAM > 90%)"}).encode())
                return

            async with httpx.AsyncClient(timeout=120.0) as client:
                response = await client.post(f"{OLLAMA_URL}/api/generate", json=payload)
                response.raise_for_status()
                resp_json = response.json()
                text = resp_json.get("response", "").strip()
                tokens = resp_json.get("eval_count", 0)
                
                self.total_tokens += tokens
                logger.info(f"✅ Generated {tokens} tokens via {model}.")

                await self.nc.publish(reply_to, json.dumps({
                    "text": text,
                    "provider": "ollama",
                    "model": model,
                    "node_id": NODE_ID,
                    "tokens": tokens
                }).encode())

        except Exception as e:
            logger.error(f"Error processing inference: {e}")
            if msg.reply:
                await self.nc.publish(msg.reply, json.dumps({"error": str(e)}).encode())

    async def broadcast_heartbeat(self):
        while self.running:
            if self.nc.is_connected:
                ram = psutil.virtual_memory().percent
                cpu = psutil.cpu_percent()
                payload = {
                    "node_id": NODE_ID,
                    "status": "ready" if ram < 85 else "pressure",
                    "capabilities": ["inference.ollama"],
                    "metrics": {
                        "ram": ram,
                        "cpu": cpu,
                        "total_tokens": self.total_tokens
                    },
                    "timestamp": datetime.now(timezone.utc).isoformat()
                }
                await self.nc.publish("heiwa.node.heartbeat", json.dumps(payload).encode())
            await asyncio.sleep(10)

    async def run(self):
        if not await self.connect():
            return

        # Listen specifically for cloud requests routed to the GPU node
        await self.nc.subscribe("heiwa.inference.request", cb=self._handle_inference_request)
        logger.info(f"🎧 Listening for NATS inference requests on 'heiwa.inference.request'")
        
        asyncio.create_task(self.broadcast_heartbeat())

        while True:
            await asyncio.sleep(1)

if __name__ == '__main__':
    node = InferenceNode()
    try:
        asyncio.run(node.run())
    except KeyboardInterrupt:
        logger.info("Shutting down Macbook Inference Node.")
