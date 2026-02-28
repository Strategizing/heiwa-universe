# libs/heiwa_sdk/agents.py
import asyncio
import json
import os
from abc import ABC, abstractmethod
from heiwa_sdk.nervous_system import HeiwaNervousSystem
from swarm_hub.config import IDENTITY

class HeiwaAgent(ABC):
    """
    Base class for all Heiwa Sovereign Agents.
    Handles NATS connectivity, identity context, and Moltbook logging.
    """

    def __init__(self, name: str, capabilities: list):
        self.name = name
        self.capabilities = capabilities
        self.nerve = HeiwaNervousSystem()
        self.identity = {}

    async def connect_to_spine(self):
        """Connects to the NATS Nervous System."""
        await self.nerve.connect()
        print(f"[{self.name}] Integrated into Heiwa Nervous System.")
        # Load Identity Context
        self._load_identity()

    def _load_identity(self):
        """Loads the Sovereign Identity from the canonical config (identity.yaml)."""
        self.identity = IDENTITY
        print(f"[{self.name}] Identity loaded: {self.identity.get('codename', 'Unknown')}")

    async def publish_to_moltbook(self, content: str, status: str = "LOG"):
        """Sends updates to the Messenger Agent for Discord output."""
        payload = {
            "agent": self.name,
            "status": status,
            "content": content,
            "metadata": {"stack": self.capabilities}
        }
        await self.nerve.publish_directive("heiwa.moltbook.logs", payload)

    async def listen_for_directives(self, subject: str):
        """Subscribes to a NATS subject and routes to execute_directive."""
        async def handler(msg):
            data = json.loads(msg.data.decode())
            print(f"[{self.name}] Directive Received: {data}")
            await self.execute_directive(data)
            await msg.ack()
        
        await self.nerve.subscribe_worker(subject, handler)
        print(f"[{self.name}] Listening on: {subject}")

    @abstractmethod
    async def execute_directive(self, directive: dict):
        """
        The specific logic for this agent.
        Must be implemented by subclasses (OpenClaw, Codex, OpenCode).
        """
        pass

    async def run(self, listen_subject: str):
        """Main loop for the agent."""
        await self.connect_to_spine()
        await self.listen_for_directives(listen_subject)
        # Keep alive
        while True:
            await asyncio.sleep(1)