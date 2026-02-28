import asyncio
import json
import logging
import re
import time
from abc import ABC, abstractmethod
from typing import Any, Dict, Optional
import nats
from nats.aio.client import Client as NATSClient

from fleets.hub.config import IDENTITY
from fleets.hub.protocol import Subject, Payload

# Configure Logging
logging.basicConfig(level=logging.INFO)
logger = logging.getLogger("AgentBase")

_NATS_URL_CRED_RE = re.compile(r"(://)[^:@/]+:[^@/]+@")


def _redact_nats_url(url: str) -> str:
    return _NATS_URL_CRED_RE.sub(r"\1<user>:<redacted>@", url or "")

from libs.heiwa_sdk.vault import InstanceVault

class BaseAgent(ABC):
    """
    Abstract Base Class for all Heiwa Agents.
    Enforces standardized NATS communication and lifecycle management.
    """
    def __init__(self, name: str = "BaseAgent"):
        self.name = name
        self.id = IDENTITY.get("uuid", "unknown")
        self.nc: Optional[NATSClient] = None
        self.running = False
        self.vault = InstanceVault()

    async def connect(self, nats_url: str = None, max_retries: int = 10, retry_delay: int = 5):
        """Connects to the NATS Swarm with retry logic."""
        import os
        url = nats_url or os.getenv("NATS_URL", "nats://localhost:4222")
        
        for attempt in range(1, max_retries + 1):
            try:
                self.nc = await nats.connect(url, connect_timeout=10)
                logger.info(f"✅ [{self.name}] Connected to NATS at {_redact_nats_url(url)}")
                self.running = True
                return
            except Exception as e:
                if attempt == max_retries:
                    logger.warning(f"⚠️ [{self.name}] NATS unavailable after {max_retries} attempts: {e}. Running in standalone mode.")
                    self.nc = None
                    self.running = True
                else:
                    logger.info(f"🔄 [{self.name}] NATS connection failed (Attempt {attempt}/{max_retries}), retrying in {retry_delay}s...")
                    await asyncio.sleep(retry_delay)

    async def speak(self, subject: Subject, data: Dict[str, Any]):
        """Publish a message to the Swarm."""
        if not self.nc:
            logger.debug(f"⚠️ [{self.name}] Cannot speak; NATS not connected.")
            return

        payload = {
            Payload.SENDER_ID: self.id,
            Payload.TIMESTAMP: time.time(),
            Payload.TYPE: subject.name,
            Payload.DATA: data
        }
        
        # NATS requires bytes
        await self.nc.publish(subject.value, json.dumps(payload).encode())
        logger.debug(f"📢 [{self.name}] Published to {subject.value}")

    async def think(self, thought: str, task_id: str = None, context: Dict[str, Any] = None, encrypt: bool = False):
        """Broadcast a reasoning thought to the swarm."""
        content = thought
        if encrypt:
            content = f"[ENCRYPTED]: {self.vault.encrypt(thought)}"

        await self.speak(Subject.LOG_THOUGHT, {
            "agent": self.name,
            "task_id": task_id,
            "content": content,
            "encrypted": encrypt,
            "context": context or {}
        })
        logger.info(f"🧠 [{self.name}] Thought: {thought[:100]}...")

    async def listen(self, subject: Subject, callback):
        """Subscribe to a Swarm subject."""
        if not self.nc:
            return

        async def wrapped_callback(msg):
            data = json.loads(msg.data.decode())
            await callback(data)

        await self.nc.subscribe(subject.value, cb=wrapped_callback)
        logger.info(f"👂 [{self.name}] Listening on {subject.value}")

    @abstractmethod
    async def run(self):
        """The main execution loop. Must be implemented by subclasses."""
        pass

    async def shutdown(self):
        """Graceful shutdown."""
        self.running = False
        if self.nc:
            await self.nc.close()
        logger.info(f"💤 [{self.name}] Shutdown complete.")


class ProposalAgent(BaseAgent):
    """
    Base for agents that process tasks and produce proposals.
    Used by OpenClaw (Strategist) and Codex (Builder).
    """
    def __init__(self, name: str, listen_subject: str):
        super().__init__(name=name)
        self.listen_subject = listen_subject

    @abstractmethod
    async def process(self, task_data: dict) -> tuple[str, str]:
        """Process a task, return (result_content, result_type)."""
        pass

    async def run(self):
        await self.connect()
        
        if self.nc:
            await self.nc.subscribe(self.listen_subject, cb=self._handle_task)

        logger.info(f"🤖 [{self.name}] Active.")
        # Keep the agent alive indefinitely
        while True:
            await asyncio.sleep(1)

    async def _handle_task(self, msg):
        task_data: Dict[str, Any] = {}
        try:
            payload = json.loads(msg.data.decode())
            task_data = payload.get("data", payload)

            result, rtype = await self.process(task_data)
            logger.info(f"[{self.name}] Produced {rtype}: {str(result)[:50]}...")

            # Emit structured log event so Messenger can post results in Discord.
            if self.nc:
                await self.speak(
                    Subject.LOG_INFO,
                    {
                        "status": "PASS",
                        "agent": self.name,
                        "task_id": task_data.get("task_id"),
                        "intent_class": task_data.get("intent_class", "generic"),
                        "requested_by": task_data.get("requested_by"),
                        "response_channel_id": task_data.get("response_channel_id"),
                        "response_thread_id": task_data.get("response_thread_id"),
                        "result_type": rtype,
                        "content": str(result)[:1800],
                    },
                )
        except Exception as e:
            logger.error(f"[{self.name}] Error processing task: {e}")
            if self.nc:
                await self.speak(
                    Subject.LOG_ERROR,
                    {
                        "status": "FAIL",
                        "agent": self.name,
                        "task_id": task_data.get("task_id"),
                        "intent_class": task_data.get("intent_class", "generic"),
                        "response_channel_id": task_data.get("response_channel_id"),
                        "response_thread_id": task_data.get("response_thread_id"),
                        "result_type": "error",
                        "content": str(e),
                    },
                )
