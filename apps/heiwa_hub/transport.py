"""
Hub-native transport layer.

Replaces NATS with two transport backends that share one interface:
  - LocalBusTransport: asyncio-based in-process pub/sub for Railway agents
  - WorkerSessionManager: outbound WebSocket delivery for remote Mac/WSL workers

Agents call speak() and listen() on the transport exactly as before.
The transport decides whether delivery is local or remote.
"""

import asyncio
import json
import logging
import time
from collections import defaultdict
from typing import Any, Callable, Coroutine, Dict, Optional

from heiwa_protocol.protocol import Subject, Payload

logger = logging.getLogger("Hub.Transport")

# Type alias for event callbacks
EventCallback = Callable[[Dict[str, Any]], Coroutine[Any, Any, None]]


class LocalBusTransport:
    """
    In-process event bus for agents running inside the Railway hub.

    Replaces NATS pub/sub for co-located agents. Zero network overhead.
    Subscribers receive events via asyncio tasks — non-blocking fanout.
    """

    def __init__(self):
        self._subscribers: dict[str, list[EventCallback]] = defaultdict(list)
        self._started = True

    async def publish(self, subject: Subject, data: Dict[str, Any], sender_id: str = "hub"):
        """Publish an event to all local subscribers of this subject.

        Callbacks are dispatched sequentially within a subject to preserve
        ordering (e.g. STATUS before EXEC_RESULT), but each publish call
        returns immediately — the dispatch runs in a background task so the
        publisher is never blocked by slow subscribers.
        """
        envelope = {
            Payload.SENDER_ID: sender_id,
            Payload.TIMESTAMP: time.time(),
            Payload.TYPE: subject.name,
            Payload.DATA: data,
        }
        key = subject.value
        # Snapshot the subscriber list so mutations during dispatch are safe
        callbacks = list(self._subscribers.get(key, []))
        if callbacks:
            asyncio.create_task(self._ordered_dispatch(callbacks, envelope, key))

    async def _ordered_dispatch(self, callbacks: list[EventCallback], data: Dict[str, Any], subject: str):
        """Dispatch to all callbacks in order, awaiting each one."""
        for cb in callbacks:
            await self._safe_dispatch(cb, data, subject)

    async def subscribe(self, subject: Subject, callback: EventCallback):
        """Register a callback for a subject."""
        self._subscribers[subject.value].append(callback)
        logger.debug("LocalBus: subscribed to %s", subject.value)

    def unsubscribe(self, subject: Subject, callback: EventCallback):
        """Remove a specific callback from a subject."""
        try:
            self._subscribers[subject.value].remove(callback)
        except ValueError:
            pass

    async def request(
        self, subject: Subject, data: Dict[str, Any], sender_id: str = "hub", timeout: float = 5.0
    ) -> Optional[Dict[str, Any]]:
        """
        Request-reply pattern over the local bus.
        Publishes to subject and waits for a single reply via a temporary
        reply subject.  Used for Spine -> Broker enrichment when both are
        in-process (kept for interface compatibility but Spine should
        prefer direct BrokerEnrichmentService.enrich() calls).
        """
        reply_event: asyncio.Event = asyncio.Event()
        reply_data: list[Dict[str, Any]] = []

        async def _capture(data: Dict[str, Any]):
            reply_data.append(data)
            reply_event.set()

        reply_subject_key = f"_reply.{subject.value}.{id(reply_event)}"
        self._subscribers[reply_subject_key].append(_capture)

        envelope = {
            Payload.SENDER_ID: sender_id,
            Payload.TIMESTAMP: time.time(),
            Payload.TYPE: subject.name,
            Payload.DATA: data,
            "_reply_subject": reply_subject_key,
        }
        for cb in self._subscribers.get(subject.value, []):
            asyncio.create_task(self._safe_dispatch(cb, envelope, subject.value))

        try:
            await asyncio.wait_for(reply_event.wait(), timeout=timeout)
            return reply_data[0] if reply_data else None
        except asyncio.TimeoutError:
            return None
        finally:
            self._subscribers[reply_subject_key].remove(_capture)

    async def reply(self, reply_subject_key: str, data: Dict[str, Any]):
        """Send a reply to a request-reply exchange."""
        for cb in self._subscribers.get(reply_subject_key, []):
            asyncio.create_task(self._safe_dispatch(cb, data, reply_subject_key))

    @staticmethod
    async def _safe_dispatch(cb: EventCallback, data: Dict[str, Any], subject: str):
        try:
            await cb(data)
        except Exception as exc:
            logger.error("LocalBus dispatch error on %s: %s", subject, exc)

    async def shutdown(self):
        self._started = False
        self._subscribers.clear()


class WorkerSessionManager:
    """
    Manages outbound WebSocket connections from remote workers (Mac/WSL).

    Workers call `WS /ws/worker` on the hub. This manager tracks active
    sessions, pushes task assignments, and receives results/heartbeats.
    """

    def __init__(self):
        # worker_id -> WebSocket instance
        self._sessions: Dict[str, Any] = {}
        # worker_id -> capabilities dict
        self._capabilities: Dict[str, Dict[str, Any]] = {}
        # worker_id -> last heartbeat timestamp
        self._heartbeats: Dict[str, float] = {}

    def register(self, worker_id: str, ws: Any, capabilities: Dict[str, Any] = None):
        """Register a worker WebSocket session."""
        self._sessions[worker_id] = ws
        self._capabilities[worker_id] = capabilities or {}
        self._heartbeats[worker_id] = time.time()
        logger.info("Worker registered: %s (capabilities: %s)", worker_id, list((capabilities or {}).keys()))

    def unregister(self, worker_id: str):
        """Remove a worker session."""
        self._sessions.pop(worker_id, None)
        self._capabilities.pop(worker_id, None)
        self._heartbeats.pop(worker_id, None)
        logger.info("Worker unregistered: %s", worker_id)

    def heartbeat(self, worker_id: str):
        """Update heartbeat timestamp for a worker."""
        self._heartbeats[worker_id] = time.time()

    def get_active_workers(self, max_stale_sec: float = 60.0) -> list[str]:
        """Return worker IDs with recent heartbeats."""
        now = time.time()
        return [
            wid for wid, ts in self._heartbeats.items()
            if now - ts < max_stale_sec and wid in self._sessions
        ]

    def get_worker_for_runtime(self, target_runtime: str) -> Optional[str]:
        """Find an active worker matching the target runtime.

        Matches against:
          - worker_id itself (e.g. ``macbook@heiwa-node-a``)
          - the ``node_id`` capability field
          - the ``runtime`` capability field
          - substring match on worker_id (e.g. ``macbook`` matches ``macbook@heiwa-node-a``)
          - ``any`` / ``both`` match any active worker
        """
        target = target_runtime.lower()
        if target in {"any", "both"}:
            active = self.get_active_workers()
            return active[0] if active else None

        for wid in self.get_active_workers():
            caps = self._capabilities.get(wid, {})
            node_id = str(caps.get("node_id", "")).lower()
            runtime = str(caps.get("runtime", "")).lower()
            wid_lower = wid.lower()
            if (
                target == wid_lower
                or target == node_id
                or target == runtime
                or target in wid_lower
                or target in node_id
            ):
                return wid
        return None

    async def push_task(self, worker_id: str, task_payload: Dict[str, Any]) -> bool:
        """Push a task assignment to a connected worker."""
        ws = self._sessions.get(worker_id)
        if not ws:
            return False
        try:
            await ws.send_json({"type": "task_assignment", "data": task_payload})
            return True
        except Exception as exc:
            logger.error("Failed to push task to worker %s: %s", worker_id, exc)
            self.unregister(worker_id)
            return False

    async def broadcast(self, message: Dict[str, Any]):
        """Send a message to all connected workers."""
        dead = []
        for wid, ws in self._sessions.items():
            try:
                await ws.send_json(message)
            except Exception:
                dead.append(wid)
        for wid in dead:
            self.unregister(wid)


# Singleton instances for the hub process
_bus: Optional[LocalBusTransport] = None
_workers: Optional[WorkerSessionManager] = None


def get_bus() -> LocalBusTransport:
    global _bus
    if _bus is None:
        _bus = LocalBusTransport()
    return _bus


def get_worker_manager() -> WorkerSessionManager:
    global _workers
    if _workers is None:
        _workers = WorkerSessionManager()
    return _workers
