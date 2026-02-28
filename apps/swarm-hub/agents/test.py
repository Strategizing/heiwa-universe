import asyncio
import logging
import sys
import os

# Add the project root to sys.path to ensure absolute imports work
sys.path.append(os.path.abspath(os.path.join(os.path.dirname(__file__), "../../../")))

from swarm_hub.agents.base import BaseAgent
from protocol.protocol import Subject

# Configure local logging to see what's happening in the terminal
logging.basicConfig(level=logging.INFO, format='%(asctime)s - %(name)s - %(levelname)s - %(message)s')
logger = logging.getLogger("TestAgent")

class HeartbeatAgent(BaseAgent):
    async def run(self):
        # 1. Connect to the Swarm
        # Note: We use the default localhost NATS unless overridden
        await self.connect()

        # 2. Setup a Listener (Reflex)
        # If anyone says "heiwa.core.dispatch", we print it.
        await self.listen(Subject.DISPATCH_TASK, self.handle_dispatch)

        # 3. The Main Loop (Pulse)
        logger.info("💓 Heartbeat Protocol Initiated.")
        try:
            while self.running:
                # Construct status data
                status = {
                    "cpu": "nominal", # Placeholder for real stats
                    "memory": "nominal",
                    "task": "idling"
                }
                
                # Speak to the network
                await self.speak(Subject.NODE_HEARTBEAT, status)
                
                # Wait 5 seconds
                await asyncio.sleep(5)
        except KeyboardInterrupt:
            await self.shutdown()

    async def handle_dispatch(self, data):
        """Callback for when we receive a task."""
        logger.info(f"📨 RECEIVED DISPATCH: {data}")

if __name__ == "__main__":
    # Bootstrap and run
    agent = HeartbeatAgent(name="Alpha-Test-Node")
    try:
        asyncio.run(agent.run())
    except KeyboardInterrupt:
        # Handle manual Ctrl+C cleanly
        asyncio.run(agent.shutdown())