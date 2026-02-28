import asyncio
import logging
import sys
import os

# Ensure project root is on sys.path for direct script execution
sys.path.insert(0, os.path.abspath(os.path.join(os.path.dirname(__file__), "../../")))

from swarm_hub.agents.spine import SpineAgent
from swarm_hub.agents.messenger import MessengerAgent
from swarm_hub.agents.executor import ExecutorAgent
from swarm_hub.agents.telemetry import TelemetryAgent
from swarm_hub.health import start_health_server

# Configure Global Logging
logging.basicConfig(
    level=logging.INFO, 
    format='%(asctime)s - %(name)s - %(levelname)s - %(message)s'
)

async def main():
    print("--- HEIWA LIMITED: CORE COLLECTIVE ---")

    # Instantiate core fleet components.
    spine = SpineAgent()
    executor = ExecutorAgent()
    telemetry = TelemetryAgent()
    tasks = [spine.run(), executor.run(), telemetry.run(), start_health_server()]

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