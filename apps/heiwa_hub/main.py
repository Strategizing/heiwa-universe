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
from heiwa_hub.agents.executor import ExecutorAgent
from heiwa_hub.agents.messenger import MessengerAgent
from heiwa_hub.agents.telemetry import TelemetryAgent
from heiwa_hub.mcp_server import app as hub_app

logging.basicConfig(
    level=logging.INFO,
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s'
)


async def _start_server(port: int):
    import uvicorn
    logger = logging.getLogger("Hub.Server")
    logger.info("Heiwa Hub booting on port %s...", port)
    config = uvicorn.Config(hub_app, host="0.0.0.0", port=port, log_level="info")
    server = uvicorn.Server(config)
    await server.serve()


async def main():
    splash = """
    █░█ █▀▀ █ █░█░█ ▄▀█
    █▀█ ██▄ █ ▀▄▀▄▀ █▀█
    [ HEIWA HUB v3.0 — Railway + SpacetimeDB ]
    """
    logger = logging.getLogger("Hub")
    print(splash)

    # Initialize persistence
    from heiwa_sdk.db import Database
    db = Database()
    if db.state_backend != "spacetimedb":
        asyncio.create_task(asyncio.to_thread(db.init_db))
    else:
        logger.info("[BOOT] SpacetimeDB selected as authoritative state layer.")

    # Register MCP servers from ai_router.json
    async def _register_mcp_servers():
        import json
        router_path = ROOT / "config/swarm/ai_router.json"
        if router_path.exists():
            config = json.loads(router_path.read_text())
            for name in config.get("mcp_servers", {}):
                logger.info("[MCP] Registered '%s' via ai_router.json", name)

    asyncio.create_task(_register_mcp_servers())

    # Boot agents — all use local bus transport (no NATS)
    try:
        spine = SpineAgent()
        executor = ExecutorAgent()
        telemetry = TelemetryAgent()
    except Exception as e:
        logger.error("[BOOT_FATAL] Failed to instantiate core agents: %s", e)
        sys.exit(1)

    port = int(os.getenv("PORT", "8080"))
    tasks = [
        asyncio.create_task(spine.run()),
        asyncio.create_task(executor.run()),
        asyncio.create_task(telemetry.run()),
        asyncio.create_task(_start_server(port=port)),
    ]

    # Broker enrichment is now a direct service call from Spine — no separate agent.

    # Messenger is optional: run only when Discord token exists.
    messenger_mode = os.getenv("HEIWA_ENABLE_MESSENGER", "auto").strip().lower()
    has_discord_token = bool(os.getenv("DISCORD_TOKEN") or os.getenv("DISCORD_BOT_TOKEN"))
    if messenger_mode == "true" or (messenger_mode == "auto" and has_discord_token):
        messenger = MessengerAgent()
        tasks.append(asyncio.create_task(messenger.run()))
    else:
        logger.info("[BOOT] Messenger disabled (no Discord token).")

    logger.info("[BOOT] All services dispatched (%d tasks).", len(tasks))
    await asyncio.gather(*tasks)


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("\nShutting down...")
