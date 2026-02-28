import asyncio
import json
import logging
import os
import sys
import time
import psutil
from pathlib import Path

# Ensure runtime libs can be imported
ROOT = Path(__file__).resolve().parents[2]
RUNTIME_ROOT = ROOT / "runtime"
if str(RUNTIME_ROOT) not in sys.path:
    sys.path.insert(0, str(RUNTIME_ROOT))

from fleets.hub.agents.base import BaseAgent
from fleets.hub.protocol import Subject

logger = logging.getLogger("MiniThinker")

class MiniThinker(BaseAgent):
    """
    Lightweight resource monitor for Node hardware.
    Periodically polls system stats and broadcasts them to the mesh.
    """
    def __init__(self):
        node_id = os.getenv("HEIWA_NODE_ID", "macbook@heiwa-agile")
        super().__init__(name=f"mini-thinker-{node_id}")
        self.node_id = node_id
        self.poll_interval = 30 # seconds

    async def run(self):
        await self.connect()
        logger.info(f"🧠 Mini Thinker active on {self.node_id}. Polling resources...")
        
        while self.running:
            try:
                stats = self.collect_stats()
                await self.speak(Subject.NODE_TELEMETRY, stats)
                
                # Low-intensity thought if resources are high
                if stats["cpu_pct"] > 80 or stats["ram_pct"] > 85:
                    await self.think(
                        f"Resource alert on {self.node_id}: CPU {stats['cpu_pct']}% | RAM {stats['ram_pct']}%",
                        context={"stats": stats}
                    )
            except Exception as e:
                logger.error(f"Failed to poll stats: {e}")
            
            await asyncio.sleep(self.poll_interval)

    def collect_stats(self):
        """Collect local hardware metrics."""
        return {
            "node_id": self.node_id,
            "cpu_pct": psutil.cpu_percent(interval=1),
            "ram_pct": psutil.virtual_memory().percent,
            "ram_used_gb": round(psutil.virtual_memory().used / (1024**3), 2),
            "ram_total_gb": round(psutil.virtual_memory().total / (1024**3), 2),
            "disk_pct": psutil.disk_usage('/').percent,
            "boot_time": psutil.boot_time(),
            "timestamp": time.time()
        }

if __name__ == "__main__":
    logging.basicConfig(level=logging.INFO)
    agent = MiniThinker()
    asyncio.run(agent.run())
