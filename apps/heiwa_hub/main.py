import asyncio
import logging
import sys
import os
from pathlib import Path

# Ensure monorepo roots are on sys.path
ROOT = Path(__file__).resolve().parents[2]
if str(ROOT / "packages/heiwa_sdk") not in sys.path:
    sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))
if str(ROOT / "packages/heiwa_protocol") not in sys.path:
    sys.path.insert(0, str(ROOT / "packages/heiwa_protocol"))
    sys.path.insert(0, str(ROOT / "packages/heiwa_identity"))
    sys.path.insert(0, str(ROOT / "packages/heiwa_ui"))
if str(ROOT / "apps") not in sys.path:
    sys.path.insert(0, str(ROOT / "apps"))

from heiwa_sdk.config import load_swarm_env
load_swarm_env()

from heiwa_hub.agents.spine import SpineAgent
from heiwa_hub.agents.messenger import MessengerAgent
from heiwa_hub.agents.executor import ExecutorAgent
from heiwa_hub.agents.telemetry import TelemetryAgent
from heiwa_hub.mcp_server import app as hub_app

# Configure Global Logging
logging.basicConfig(
    level=logging.INFO, 
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s'
)

async def _start_server(port: int):
    import uvicorn
    logger = logging.getLogger("Hub.Server")
    logger.info("📡 Heiwa Hub Server booting on port %s...", port)
    config = uvicorn.Config(hub_app, host="0.0.0.0", port=port, log_level="info")
    server = uvicorn.Server(config)
    await server.serve()

async def main():
    splash = """
    █░█ █▀▀ █ █░█░█ ▄▀█
    █▀█ ██▄ █ ▀▄▀▄▀ █▀█ 
    [ HEIWA ENTERPRISE HUB v2.1 ]
    """
    print(splash)
    logger = logging.getLogger("Hub")
    logger.info("Initializing SOTA AI-Dentity Sovereign Mesh...")

    # 0. Initialize Persistence
    from heiwa_sdk.db import Database
    db = Database()
    db.init_db()

    # Instantiate core fleet components.
    spine = SpineAgent()
    executor = ExecutorAgent()
    telemetry = TelemetryAgent()
    
    port = int(os.getenv("PORT", "8080"))
    tasks = [spine.run(), executor.run(), telemetry.run(), _start_server(port=port)]

    # Messenger is optional: run only when token exists (or explicitly forced).
    messenger_mode = os.getenv("HEIWA_ENABLE_MESSENGER", "auto").strip().lower()
    has_discord_token = bool(os.getenv("DISCORD_TOKEN") or os.getenv("DISCORD_BOT_TOKEN"))
    messenger_enabled = messenger_mode == "true" or (messenger_mode == "auto" and has_discord_token)

    if messenger_enabled:
        messenger = MessengerAgent()
        tasks.append(messenger.run())
    else:
        print("[INFO] Messenger disabled (set HEIWA_ENABLE_MESSENGER=true to force enable).")

    # Run all enabled services in parallel.
    await asyncio.gather(*tasks)

if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("\n🛑 Collective shutting down...")
