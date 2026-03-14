import asyncio
import json
import logging
import time
from abc import ABC, abstractmethod
from typing import Any, Dict, Optional

from heiwa_identity.node import load_node_identity
from heiwa_protocol.protocol import Subject, Payload
from heiwa_sdk.config import settings
from heiwa_sdk.security import redact_text
from heiwa_sdk.vault import InstanceVault
from heiwa_hub.transport import LocalBusTransport, get_bus

IDENTITY = load_node_identity()

logging.basicConfig(level=logging.INFO)
logger = logging.getLogger("AgentBase")


class BaseAgent(ABC):
    """
    Abstract base for all Heiwa agents.

    Uses LocalBusTransport for in-process event delivery on Railway.
    speak() and listen() have the same signatures as before so agent
    code stays unchanged.
    """

    def __init__(self, name: str = "BaseAgent"):
        self.name = name
        self.id = IDENTITY.get("uuid", "unknown")
        self.bus: LocalBusTransport = get_bus()
        self.running = False
        self.vault = InstanceVault()

    async def start(self):
        """Mark the agent as running and begin the heartbeat loop."""
        self.running = True
        asyncio.create_task(self._telemetry_heartbeat())
        logger.info("[%s] Started on local bus.", self.name)

    async def _telemetry_heartbeat(self):
        """Broadcast node resource usage every 30 seconds."""
        try:
            import psutil
        except ImportError:
            return
        while self.running:
            try:
                stats = {
                    "node_id": self.id,
                    "agent_name": self.name,
                    "cpu_pct": psutil.cpu_percent(),
                    "ram_pct": psutil.virtual_memory().percent,
                    "ram_used_gb": round(psutil.virtual_memory().used / (1024**3), 2),
                    "ram_total_gb": round(psutil.virtual_memory().total / (1024**3), 2),
                    "timestamp": time.time(),
                    "status": "ONLINE",
                }
                await self.speak(Subject.NODE_HEARTBEAT, stats)
                await self.speak(Subject.NODE_TELEMETRY, stats)
            except Exception as e:
                logger.debug("Telemetry heartbeat failed for %s: %s", self.name, e)
            await asyncio.sleep(30)

    async def speak(self, subject: Subject, data: Dict[str, Any]):
        """Publish an event to the local bus."""
        await self.bus.publish(subject, data, sender_id=self.id)

    async def think(self, thought: str, task_id: str = None, context: Dict[str, Any] = None, encrypt: bool = False):
        """Broadcast a reasoning thought."""
        content = thought
        if encrypt:
            content = f"[ENCRYPTED]: {self.vault.encrypt(thought)}"
        await self.speak(Subject.LOG_THOUGHT, {
            "agent": self.name,
            "task_id": task_id,
            "content": content,
            "encrypted": encrypt,
            "context": context or {},
        })
        logger.info("[%s] Thought: %s...", self.name, redact_text(thought[:100]))

    async def listen(self, subject: Subject, callback):
        """Subscribe to a subject on the local bus."""
        await self.bus.subscribe(subject, callback)
        logger.info("[%s] Listening on %s", self.name, subject.value)

    @abstractmethod
    async def run(self):
        """Main execution loop. Implemented by subclasses."""
        pass

    async def shutdown(self):
        """Graceful shutdown."""
        self.running = False
        logger.info("[%s] Shutdown complete.", self.name)


class ProposalAgent(BaseAgent):
    """
    Base for agents that process tasks and produce proposals.
    """

    def __init__(self, name: str, listen_subject: str):
        super().__init__(name=name)
        self.listen_subject = listen_subject

    @abstractmethod
    async def process(self, task_data: dict) -> tuple[str, str]:
        """Process a task, return (result_content, result_type)."""
        pass

    async def run(self):
        await self.start()
        await self.bus.subscribe(
            Subject(self.listen_subject) if isinstance(self.listen_subject, str) else self.listen_subject,
            self._handle_task,
        )
        logger.info("[%s] Active.", self.name)
        while self.running:
            await asyncio.sleep(1)

    async def _handle_task(self, data: Dict[str, Any]):
        task_data: Dict[str, Any] = {}
        try:
            task_data = data.get("data", data)
            result, rtype = await self.process(task_data)
            logger.info("[%s] Produced %s: %s...", self.name, rtype, str(result)[:50])
            await self.speak(Subject.LOG_INFO, {
                "status": "PASS",
                "agent": self.name,
                "task_id": task_data.get("task_id"),
                "intent_class": task_data.get("intent_class", "generic"),
                "requested_by": task_data.get("requested_by"),
                "response_channel_id": task_data.get("response_channel_id"),
                "response_thread_id": task_data.get("response_thread_id"),
                "result_type": rtype,
                "content": str(result)[:1800],
            })
        except Exception as e:
            logger.error("[%s] Error processing task: %s", self.name, e)
            await self.speak(Subject.LOG_ERROR, {
                "status": "FAIL",
                "agent": self.name,
                "task_id": task_data.get("task_id"),
                "intent_class": task_data.get("intent_class", "generic"),
                "response_channel_id": task_data.get("response_channel_id"),
                "response_thread_id": task_data.get("response_thread_id"),
                "result_type": "error",
                "content": str(e),
            })
