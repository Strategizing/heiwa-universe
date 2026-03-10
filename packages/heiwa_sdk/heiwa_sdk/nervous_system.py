# libs/heiwa_sdk/nervous_system.py
import asyncio
import json
import logging
import os
import nats
from nats.errors import ConnectionClosedError, TimeoutError, NoServersError

# Configure logging
logging.basicConfig(level=logging.INFO)
logger = logging.getLogger("NervousSystem")

class HeiwaNervousSystem:
    """
    The Nervous System (NATS) for Heiwa Limited.
    Connects the Spine (Brain) to the Muscle (Local).
    """

    def __init__(self, nats_url=None):
        # PRIORITY 1: passed arg
        # PRIORITY 2: NATS_URL from env (Railway)
        # FALLBACK: Localhost for dev
        self.nats_url = nats_url or os.getenv("NATS_URL") or "nats://localhost:4222"
        self.nc = None
        self.js = None

    async def connect(self):
        """Establishes a resilient connection to the Sovereign Nerve Center."""
        try:
            logger.info(f"Connecting to Nerve Center at {self.nats_url}...")
            # Robust connection options for "Sovereign Reconnect"
            self.nc = await nats.connect(
                self.nats_url,
                reconnect_time_wait=2,
                max_reconnect_attempts=-1  # Infinite retry
            )
            # Initialize JetStream for persistent 'overnight' tasks
            self.js = self.nc.jetstream()
            logger.info("Connected to Nerve Center (JetStream Enabled).")
        except Exception as e:
            logger.error(f"Failed to connect to NATS: {e}")
            raise e

    async def disconnect(self):
        """Drains and closes the connection."""
        if self.nc:
            await self.nc.drain()
            logger.info("Nervous System drained.")

    async def publish_directive(self, subject: str, data: dict):
        """
        Spine calls this to issue a command.
        """
        if not self.js:
            raise ConnectionError("Nervous System not connected. Call connect() first.")

        payload = json.dumps(data).encode()
        ack = await self.js.publish(subject, payload)
        logger.info(f"Directive Issued: {subject} (Seq: {ack.seq})")
        return ack

    async def subscribe_worker(self, subject: str, callback):
        """
        Muscle calls this to listen for work.
        """
        if not self.js:
            raise ConnectionError("Nervous System not connected.")

        logger.info(f"Worker Listening on: {subject}")
        # Durable consumer 'muscle_worker' ensures tasks persist if worker is offline
        await self.js.subscribe(subject, cb=callback, durable="muscle_worker", deliver_policy="all")