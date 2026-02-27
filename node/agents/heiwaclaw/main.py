import asyncio
import os
import json
import logging
import psutil
from datetime import datetime, timezone
import httpx
from nats.aio.client import Client as NATS
from watchdog.observers import Observer
from watchdog.events import FileSystemEventHandler
import discord

from ui_manager import UIManager
from swarm_bridge import SwarmBridge

# Load environment variables from .env if it exists
env_path = os.path.join(os.path.dirname(__file__), '.env')
if os.path.exists(env_path):
    with open(env_path) as f:
        for line in f:
            if line.strip() and not line.startswith('#'):
                key, val = line.strip().split('=', 1)
                os.environ[key] = val

logging.basicConfig(level=logging.INFO, format="%(asctime)s - %(name)s - %(levelname)s - %(message)s")
logger = logging.getLogger("HeiwaClaw")

# Configs
AGENT_NAME = os.getenv("AGENT_NAME", "HeiwaClaw")
NATS_URL = os.getenv("NATS_URL", "nats://localhost:4222")
DISCORD_BOT_TOKEN = os.getenv("DISCORD_BOT_TOKEN", "")
DISCORD_CHANNEL_ID = int(os.getenv("DISCORD_CHANNEL_ID", "0"))
OLLAMA_URL = os.getenv("OLLAMA_URL", "http://localhost:11434")
MODEL_PRIMARY = os.getenv("MODEL_PRIMARY", "deepseek-coder-v2:16b")
MODEL_SECONDARY = os.getenv("MODEL_SECONDARY", "qwen2.5-coder:7b")
WATCH_DIR = os.getenv("WATCH_DIR", "/Users/dmcgregsauce/heiwa/runtime/docs")
RAM_THRESHOLD_PERCENT = float(os.getenv("RAM_THRESHOLD_PERCENT", "80.0"))
CPU_THRESHOLD_PERCENT = float(os.getenv("CPU_THRESHOLD_PERCENT", "90.0"))
PERSONALITY = os.getenv("PERSONALITY", "A helpful AI assistant within the Heiwa Swarm.")
GEMINI_API_KEY = os.getenv("GEMINI_API_KEY", "")
OPENAI_API_KEY = os.getenv("OPENAI_API_KEY", "")

class AgentState:
    def __init__(self):
        self.system_paused = False
        self.scaling_active = False
        self.chat_history = []
        self.nc = NATS()
        self.metrics = {}
        self.bridge = None
        self.node_id = os.getenv("NODE_ID", f"{AGENT_NAME}-Macbook")
        self.total_tokens = 0
        self.railway_status = "Online"
        self.local_health = "Stable"
        self.active_provider = "Ollama"

    def get_snapshot(self):
        return {
            "railway": self.railway_status,
            "tokens": self.total_tokens,
            "local_health": self.local_health,
            "node_id": self.node_id,
            "provider": self.active_provider
        }

agent_state = AgentState()

intents = discord.Intents.default()
intents.message_content = True
client = discord.Client(intents=intents)

class DebouncedEventHandler(FileSystemEventHandler):
    def __init__(self, loop, queue):
        self.loop = loop
        self.queue = queue
        self._last_events = {}

    def on_modified(self, event):
        if event.is_directory or not event.src_path.endswith(('.md', '.txt')):
            return
        now = self.loop.time()
        if event.src_path in self._last_events:
            if now - self._last_events[event.src_path] < 5.0:
                return
        self._last_events[event.src_path] = now
        asyncio.run_coroutine_threadsafe(self.queue.put(event.src_path), self.loop)

def get_ollama_process():
    for proc in psutil.process_iter(['pid', 'name']):
        if proc.info['name'] == 'ollama':
            return proc
    return None

async def check_system_health():
    ram = psutil.virtual_memory().percent
    cpu = psutil.cpu_percent(interval=0.5)
    
    if ram > 85: agent_state.local_health = "Critical"
    elif ram > 75: agent_state.local_health = "Pressure"
    else: agent_state.local_health = "Stable"
    
    return ram < RAM_THRESHOLD_PERCENT and cpu < CPU_THRESHOLD_PERCENT

async def get_optimal_model(task_type="chat"):
    ram = psutil.virtual_memory().percent
    if task_type == "chat":
        return MODEL_SECONDARY
    if ram > (RAM_THRESHOLD_PERCENT - 5.0):
        agent_state.scaling_active = True
        return MODEL_SECONDARY
    agent_state.scaling_active = False
    return MODEL_PRIMARY

async def summarize_file(filepath):
    try:
        with open(filepath, 'r', encoding='utf-8') as f:
            content = f.read()
    except Exception as e:
        logger.error(f"Could not read {filepath}: {e}")
        return

    if not await check_system_health():
        logger.warning("System overloaded, delaying summarization.")
        channel = client.get_channel(DISCORD_CHANNEL_ID)
        if channel:
            embed = UIManager.create_base_embed("Doc Update Paused", "System resources high. Skipping summarization to preserve stability.", status="overloaded", metrics=agent_state.metrics, snapshot=agent_state.get_snapshot())
            await channel.send(embed=embed)
        return

    model = await get_optimal_model(task_type="summary")
    logger.info(f"Summarizing {filepath} using {model}...")
    prompt = f"Provide a concise technical summary of the changes in this doc:\n\n{content[:2000]}"
    
    try:
        async with httpx.AsyncClient(timeout=120.0) as client_http:
            response = await client_http.post(
                f"{OLLAMA_URL}/api/generate",
                json={"model": model, "prompt": prompt, "stream": False}
            )
            response.raise_for_status()
            resp_json = response.json()
            summary = resp_json.get('response', '')
            agent_state.total_tokens += resp_json.get('eval_count', 0)
            
            channel = client.get_channel(DISCORD_CHANNEL_ID)
            if channel:
                embed = UIManager.create_task_embed(os.path.basename(filepath), prompt[:100], status="completed", result=summary, snapshot=agent_state.get_snapshot())
                embed.add_field(name="Model", value=f"`{model}`", inline=True)
                await channel.send(embed=embed)
    except Exception as e:
        logger.error(f"Failed to summarize: {e}")

async def resource_monitor_loop():
    last_alert_time = 0
    loop = asyncio.get_running_loop()
    
    while True:
        try:
            ram = psutil.virtual_memory()
            cpu = psutil.cpu_percent(interval=1)
            ollama_proc = get_ollama_process()
            ollama_usage = "N/A"
            if ollama_proc:
                try:
                    ollama_usage = f"{ollama_proc.memory_info().rss / (1024**3):.2f}GB"
                except: pass

            agent_state.metrics = {
                "node": agent_state.node_id,
                "timestamp": datetime.now(timezone.utc).isoformat(),
                "cpu": f"{cpu}%",
                "ram": f"{ram.percent}%",
                "ollama_ram": ollama_usage,
                "scaling": "ACTIVE" if agent_state.scaling_active else "OFF"
            }

            if agent_state.nc.is_connected:
                await agent_state.nc.publish(f"heiwa.node.metrics.{agent_state.node_id.lower()}", json.dumps(agent_state.metrics).encode())

            if ram.percent > RAM_THRESHOLD_PERCENT:
                agent_state.system_paused = True
                now = loop.time()
                if now - last_alert_time > 900:
                    msg = f"🚨 **{AGENT_NAME} Resource CRITICAL!** RAM: {ram.percent}% | Ollama: {ollama_usage}. Pausing inference."
                    channel = client.get_channel(DISCORD_CHANNEL_ID)
                    if channel:
                        embed = UIManager.create_base_embed("Resource Alert", msg, status="error", metrics=agent_state.metrics, snapshot=agent_state.get_snapshot())
                        await channel.send(embed=embed)
                    last_alert_time = now
            else:
                agent_state.system_paused = False

        except Exception as e:
            logger.error(f"Error in resource monitor: {e}")
        await asyncio.sleep(10)

async def file_processor_loop(queue):
    while True:
        filepath = await queue.get()
        if not agent_state.system_paused:
            await summarize_file(filepath)
        queue.task_done()

async def generate_chat_response(prompt_text):
    model = await get_optimal_model(task_type="chat")
    system_context = (
        f"Identity: {AGENT_NAME} ({agent_state.node_id}). "
        f"Personality: {PERSONALITY} "
        f"System: RAM {agent_state.metrics.get('ram')}, CPU {agent_state.metrics.get('cpu')}. "
        "Keep responses high-signal and distinct to your personality. If a task requires cloud power, say [BRIDGE_REQUEST]."
    )
    history_limit = 5 if psutil.virtual_memory().percent > 75 else 10
    messages = [{"role": "system", "content": system_context}]
    messages.extend(agent_state.chat_history[-history_limit:])
    messages.append({"role": "user", "content": prompt_text})
    
    # Try local Ollama first
    try:
        async with httpx.AsyncClient(timeout=60.0) as client_http:
            response = await client_http.post(
                f"{OLLAMA_URL}/api/chat",
                json={"model": model, "messages": messages, "stream": False}
            )
            if response.status_code == 200:
                resp_json = response.json()
                reply = resp_json.get('message', {}).get('content', '')
                agent_state.total_tokens += resp_json.get('eval_count', 0)
                agent_state.active_provider = "Ollama"
                agent_state.chat_history.append({"role": "user", "content": prompt_text})
                agent_state.chat_history.append({"role": "assistant", "content": reply})
                return reply
    except Exception as e:
        logger.warning(f"Ollama inference failed, attempting Gemini fallback: {e}")

    # Fallback to Gemini
    if GEMINI_API_KEY:
        try:
            url = f"https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent?key={GEMINI_API_KEY}"
            payload = {
                "contents": [{"parts": [{"text": f"{system_context}\n\nUser: {prompt_text}"}]}],
                "generationConfig": {"temperature": 0.2}
            }
            async with httpx.AsyncClient(timeout=30.0) as client_http:
                response = await client_http.post(url, json=payload)
                response.raise_for_status()
                data = response.json()
                if "candidates" in data and data["candidates"]:
                    reply = data["candidates"][0]["content"]["parts"][0]["text"].strip()
                    agent_state.active_provider = "Gemini"
                    # We approximate tokens for Gemini for the footer snapshot
                    agent_state.total_tokens += len(prompt_text.split()) + len(reply.split())
                    agent_state.chat_history.append({"role": "user", "content": prompt_text})
                    agent_state.chat_history.append({"role": "assistant", "content": reply})
                    return reply
        except Exception as e:
            logger.error(f"Gemini fallback failed: {e}")

    return "I encountered an error trying to process that. All local and cloud providers are currently unavailable or overloaded."

@client.event
async def on_ready():
    logger.info(f"{AGENT_NAME} active as {client.user}")
    try:
        await agent_state.nc.connect(NATS_URL)
        logger.info(f"Connected to NATS at {NATS_URL}")
        agent_state.bridge = SwarmBridge(agent_state.nc, client, DISCORD_CHANNEL_ID)
        agent_state.bridge.node_id = agent_state.node_id
        client.loop.create_task(agent_state.bridge.broadcast_capabilities())
        client.loop.create_task(agent_state.bridge.listen_for_cloud_tasks())
    except Exception as e:
        logger.warning(f"NATS Connection error: {e}")

    file_queue = asyncio.Queue()
    event_handler = DebouncedEventHandler(asyncio.get_running_loop(), file_queue)
    observer = Observer()
    observer.schedule(event_handler, WATCH_DIR, recursive=True)
    observer.start()

    client.loop.create_task(resource_monitor_loop())
    client.loop.create_task(file_processor_loop(file_queue))
    
    channel = client.get_channel(DISCORD_CHANNEL_ID)
    if channel:
        embed = UIManager.create_base_embed("Online", f"**{AGENT_NAME}** initialized. Personality core loaded: `{PERSONALITY[:50]}...`", status="online", metrics=agent_state.metrics, snapshot=agent_state.get_snapshot())
        await channel.send(embed=embed)

@client.event
async def on_message(message):
    if message.author == client.user or message.channel.id != DISCORD_CHANNEL_ID:
        return
    
    if agent_state.system_paused:
        await message.channel.send("⚠️ **Inference Paused.** Macbook resources critical.")
        return

    content = message.content.strip()

    async with message.channel.typing():
        reply = await generate_chat_response(content)
        
        # Cognitive Bridge Check
        if "[BRIDGE_REQUEST]" in reply and agent_state.bridge:
            await agent_state.bridge.bridge_to_swarm(content, str(message.author))
            return

        # Manual Cloud Trigger
        cloud_keywords = ["swarm", "cloud", "railway", "scrape", "deploy", "audit", "research"]
        if any(k in content.lower() for k in cloud_keywords) and agent_state.bridge:
            if "scrape" in content.lower():
                embed = UIManager.create_base_embed("Scrape Request", f"Initiating web research via OpenClaw: `{content}`", status="thinking", metrics=agent_state.metrics, snapshot=agent_state.get_snapshot())
                await message.channel.send(embed=embed)
            await agent_state.bridge.bridge_to_swarm(content, str(message.author))
            return

        embed = UIManager.create_base_embed(f"Response ({AGENT_NAME})", reply, status="thinking", metrics=agent_state.metrics, snapshot=agent_state.get_snapshot())
        await message.channel.send(embed=embed)

if __name__ == '__main__':
    if DISCORD_BOT_TOKEN:
        client.run(DISCORD_BOT_TOKEN)
