import asyncio
import json
import logging
import os
import time
import uuid
from nats.aio.client import Client as NATS
from ui_manager import UIManager

logger = logging.getLogger("LocalClaw.SwarmBridge")

class SwarmBridge:
    """The local-to-cloud bridge for LocalClaw and the Heiwa Swarm."""
    
    def __init__(self, nc, client, channel_id):
        self.nc = nc
        self.client = client
        self.channel_id = channel_id
        self.node_id = "LocalClaw-Macbook"
        self.capabilities = ["mcp.code_generation", "mcp.research", "mcp.strategy", "mcp.local_exec"]

    async def broadcast_capabilities(self):
        """Broadcasts LocalClaw's presence and tools to the Swarm."""
        while True:
            if self.nc.is_connected:
                payload = {
                    "node_id": self.node_id,
                    "capabilities": self.capabilities,
                    "status": "ready",
                    "timestamp": time.time()
                }
                await self.nc.publish("heiwa.mesh.capability.broadcast", json.dumps(payload).encode())
                await self.nc.publish("heiwa.node.heartbeat", json.dumps(payload).encode())
            await asyncio.sleep(30)

    async def bridge_to_swarm(self, instruction, user_name):
        """Routes a local Discord request to the Cloud Swarm via NATS."""
        task_id = f"local-bridge-{uuid.uuid4().hex[:8]}"
        
        # UI Feedback
        channel = self.client.get_channel(self.channel_id)
        if channel:
            embed = UIManager.create_task_embed(task_id, instruction, status="bridged")
            embed.add_field(name="Route", value="🔀 **Macbook -> Railway Cloud Swarm**", inline=False)
            await channel.send(embed=embed)

        payload = {
            "task_id": task_id,
            "raw_text": instruction,
            "source": "localclaw_bridge",
            "requested_by": user_name,
            "ingress_ts": time.time(),
            "target_runtime": "railway"
        }
        
        if self.nc.is_connected:
            await self.nc.publish("heiwa.tasks.ingress", json.dumps(payload).encode())
            logger.info(f"Task {task_id} bridged to Railway Swarm.")
        else:
            logger.error("Could not bridge task: NATS not connected.")
            if channel:
                await channel.send("❌ **Bridge Error**: Cloud NATS connection unavailable.")

    async def listen_for_cloud_tasks(self):
        """Listens for tasks from the Swarm that need local Macbook execution."""
        async def handle_task(msg):
            data = json.loads(msg.data.decode())
            # Logic for accepting a local task would go here
            logger.info(f"Received cloud task request: {data.get('task_id')}")
            
        if self.nc.is_connected:
            await self.nc.subscribe("heiwa.tasks.new", cb=handle_task)
            logger.info("LocalClaw Swarm Bridge: Listening for cloud tasks...")
