import asyncio
import json
import logging
import os
import time
import uuid
from typing import Dict

from nats.errors import TimeoutError

from heiwa_hub.agents.base import BaseAgent
from heiwa_protocol.protocol import Subject, Payload
from heiwa_hub.cognition.planner import LocalTaskPlanner

logger = logging.getLogger("Spine")
BROKER_ENVELOPE_VERSION = "2026-03-06"

class SpineAgent(BaseAgent):
    def __init__(self):
        super().__init__(name="heiwa-spine")
        # Registry: { "node_uuid": last_seen_timestamp }
        self.fleet_registry: Dict[str, float] = {}
        self.planner = LocalTaskPlanner()
        self.broker_enabled = os.getenv("HEIWA_ENABLE_BROKER", "true").strip().lower() == "true"

    async def run(self):
        try:
            await self.connect()
        except Exception:
            logger.warning("⚠️ NATS unavailable. Spine running in local mode.")

        # 1. Open Ears - Using Enum members
        await self.listen(Subject.NODE_HEARTBEAT, self.handle_heartbeat)
        await self.listen(Subject.CORE_REQUEST, self.handle_request)
        await self.listen(Subject.TASK_NEW, self.handle_request)

        logger.info("🧠 Spine Active. Monitoring Fleet...")

        # 2. Maintenance Loop
        try:
            while self.running:
                self._prune_registry()

                # Only log if we have an active fleet to reduce noise
                if self.fleet_registry:
                    logger.info(f"📊 Fleet Status: {len(self.fleet_registry)} active node(s).")

                await asyncio.sleep(10)
        except KeyboardInterrupt:
            await self.shutdown()

    async def handle_heartbeat(self, data: dict):
        """Update the registry when a node pulses."""
        sender = data.get("sender_id")
        # In a real scenario, you might parse 'data' for CPU load to make routing decisions

        if sender:
            if sender not in self.fleet_registry:
                logger.info(f"🆕 New Node Detected: {sender}")

            # Update timestamp
            self.fleet_registry[sender] = time.time()

    async def handle_request(self, data: dict):
        """
        Receive a Task Envelope, verify the Digital Barrier, and dispatch.
        """
        from heiwa_sdk.config import settings

        expected_token = settings.HEIWA_AUTH_TOKEN
        if not expected_token:
            logger.error("🛡️  Digital Barrier misconfigured: HEIWA_AUTH_TOKEN is not set. Dropping inbound task.")
            return

        from heiwa_hub.envelope import extract_auth_token, extract_payload, normalize_sender

        # --- DIGITAL BARRIER CHECK ---
        sender_id = normalize_sender(data)
        auth_token = extract_auth_token(data)
        if not auth_token or auth_token != expected_token:
            logger.warning(f"🛡️  Digital Barrier Breach Attempt: Invalid token from sender {sender_id}")
            await self.speak(
                Subject.TASK_STATUS,
                {
                    "accepted": False,
                    "reason": "Invalid or missing auth token.",
                    "task_id": data.get("task_id", data.get("data", {}).get("task_id", "unknown")),
                    "step_id": "spine-orchestrator",
                    "status": "BLOCKED_AUTH",
                    "message": "Invalid or missing auth token.",
                    "runtime": "spine"
                }
            )
            return

        payload = extract_payload(data)
        task_id = payload.get("task_id", f"task-{int(time.time())}")
        
        dispatch_status_code = "DISPATCHED_PLAN"
        broker_failed = False
        
        # --- AUTO-PLANNING FOR RAW REQUESTS ---
        if not payload.get("steps") and payload.get("raw_text"):
            if self.broker_enabled and self.nc:
                try:
                    payload = await self._route_via_broker(
                        payload=payload,
                        task_id=task_id,
                        sender_id=sender_id,
                    )
                    logger.info("🛰️ Broker enriched task %s.", task_id)
                except TimeoutError:
                    broker_failed = True
                    logger.warning("⚠️ Broker timeout for %s. Falling back to inline planning.", task_id)
                except Exception as e:
                    broker_failed = True
                    logger.warning("⚠️ Broker enrichment failed for %s: %s. Falling back inline.", task_id, e)

        if not payload.get("steps") and payload.get("raw_text"):
            logger.info(f"🔮 Planning raw request for task {task_id}...")
            try:
                task_plan = self.planner.plan(
                    task_id=task_id,
                    raw_text=payload.get("raw_text"),
                    requested_by=sender_id,
                    source_channel_id=payload.get("source", "cli"),
                    source_message_id=task_id,
                    response_channel_id=payload.get("response_channel_id", "cli"),
                    response_thread_id=payload.get("response_thread_id"),
                )
                payload = task_plan.to_dict()
                dispatch_status_code = "DISPATCHED_FALLBACK" if broker_failed else "DISPATCHED_PLAN"
            except Exception as e:
                logger.error(f"❌ Planning failed: {e}. Falling back to direct execution.")
                # Fallback: Create a single direct step
                payload["steps"] = [{
                    "step_id": "fallback-exec",
                    "instruction": payload.get("raw_text"),
                    "subject": Subject.TASK_EXEC.value,
                    "target_runtime": "any"
                }]
                dispatch_status_code = "DISPATCHED_FALLBACK"

        intent = payload.get("intent_class", "unknown")

        logger.info(f"📥 Received Task Envelope: {intent} (Task: {task_id})")

        # Emit an ACKNOWLEDGED status to Messenger
        ack_payload = {
            "accepted": True,
            "reason": None,
            "task_id": task_id,
            "step_id": "spine-orchestrator",
            "status": "ACKNOWLEDGED",
            "message": f"Spine has accepted Task Envelope '{task_id}' for '{intent}'. Execution bounded.",
            "runtime": "spine",
            "response_channel_id": payload.get("response_channel_id"),
            "response_thread_id": payload.get("response_thread_id"),
        }
        await self.speak(Subject.TASK_STATUS, ack_payload)
        logger.info(f"✅ Sent ACKNOWLEDGED status for {task_id}")

        steps = payload.get("steps") or []
        if not isinstance(steps, list) or not steps:
            logger.warning(f"⚠️ No steps planned for task {task_id}. Creating fallback step.")
            # Ensure we have at least one step to execute if raw_text exists
            raw_text = payload.get("raw_text") or data.get("raw_text")
            if raw_text:
                steps = [{
                    "step_id": "auto-reflex",
                    "instruction": raw_text,
                    "subject": Subject.TASK_EXEC.value,
                    "target_runtime": "any",
                    "target_tool": "ollama"
                }]
                dispatch_status_code = "DISPATCHED_FALLBACK"
            else:
                await self.speak(
                    Subject.TASK_STATUS,
                    {
                        "accepted": False,
                        "reason": "No executable content found in task.",
                        "task_id": task_id,
                        "step_id": "spine-orchestrator",
                        "status": "BLOCKED_NO_CONTENT",
                        "message": "No executable content found in task.",
                        "runtime": "spine"
                    },
                )
                return

        if not self.nc:
            await self.speak(
                Subject.TASK_STATUS,
                {
                    "accepted": False,
                    "reason": "Spine cannot dispatch because NATS is unavailable.",
                    "task_id": task_id,
                    "step_id": "spine-orchestrator",
                    "status": "FAIL",
                    "message": "Spine cannot dispatch because NATS is unavailable.",
                    "runtime": "spine",
                    "response_channel_id": payload.get("response_channel_id"),
                    "response_thread_id": payload.get("response_thread_id"),
                },
            )
            logger.warning("⚠️ Cannot dispatch %s; NATS unavailable", task_id)
            return

        for step in steps:
            if not isinstance(step, dict):
                continue
            step_subject = str(step.get("subject", Subject.TASK_EXEC.value)).strip()
            step_id = str(step.get("step_id", "unknown"))
            
            exec_payload = {
                "task_id": task_id,
                "plan_id": payload.get("plan_id"),
                "approval_id": payload.get("approval_id"),
                "step_id": step_id,
                "instruction": step.get("instruction", payload.get("raw_text", "")),
                "intent_class": payload.get("intent_class", intent),
                "risk_level": payload.get("risk_level"),
                "requested_by": payload.get("requested_by"),
                "target_runtime": step.get("target_runtime", payload.get("target_runtime", "railway")),
                "target_tool": step.get("target_tool", payload.get("target_tool", "ollama")),
                "compute_class": payload.get("compute_class"),
                "assigned_worker": payload.get("assigned_worker"),
                "requires_approval": payload.get("requires_approval"),
                "response_channel_id": payload.get("response_channel_id"),
                "response_thread_id": payload.get("response_thread_id"),
                "raw_text": payload.get("raw_text"),
                "normalization": payload.get("normalization"),
                "envelope_version": payload.get("envelope_version"),
            }

            wrapped = {
                Payload.SENDER_ID: self.id,
                Payload.TIMESTAMP: time.time(),
                Payload.TYPE: "TASK_EXEC_DISPATCH",
                Payload.DATA: exec_payload,
            }
            # Use Subject.TASK_EXEC enum member for SOTA v3.1
            await self.speak(Subject.TASK_EXEC, exec_payload)
            logger.info("📤 Dispatched %s/%s to %s", task_id, step_id, Subject.TASK_EXEC)

            await self.speak(
                Subject.TASK_STATUS,
                {
                    "accepted": True,
                    "reason": None,
                    "task_id": task_id,
                    "step_id": step_id,
                    "status": dispatch_status_code,
                    "message": f"Spine dispatched step to {step_subject}.",
                    "runtime": "spine",
                    "response_channel_id": payload.get("response_channel_id"),
                    "response_thread_id": payload.get("response_thread_id"),
                },
            )

    def _prune_registry(self):
        """Remove nodes that have gone dark."""
        now = time.time()
        timeout = 30.0 # seconds

        # Identify dead nodes
        dead_nodes = [nid for nid, last_seen in self.fleet_registry.items() if now - last_seen > timeout]

        for nid in dead_nodes:
            logger.warning(f"💀 Node Lost: {nid}")
            del self.fleet_registry[nid]

    async def _route_via_broker(self, *, payload: dict, task_id: str, sender_id: str) -> dict:
        if not self.nc:
            raise RuntimeError("NATS is unavailable.")

        request_id = f"broker-{task_id}-{uuid.uuid4().hex[:8]}"
        broker_request = {
            "request_id": request_id,
            "task_id": task_id,
            "raw_text": payload.get("raw_text", ""),
            "sender_id": sender_id,
            "source_surface": payload.get("source", "cli"),
            "response_channel_id": payload.get("response_channel_id", "cli"),
            "response_thread_id": payload.get("response_thread_id"),
            "auth_validated": True,
            "timestamp": time.time(),
            "envelope_version": BROKER_ENVELOPE_VERSION,
        }

        response = await self.nc.request(
            Subject.BROKER_ROUTE.value,
            json.dumps(broker_request).encode(),
            timeout=5.0,
        )
        routed = json.loads(response.data.decode())

        if routed.get("request_id") != request_id:
            raise ValueError(
                f"Broker request_id mismatch for {task_id}: expected {request_id}, got {routed.get('request_id')}"
            )
        if routed.get("error"):
            raise RuntimeError(f"{routed.get('error')}: {routed.get('message', '')}".strip())

        return routed

if __name__ == "__main__":
    agent = SpineAgent()
    try:
        asyncio.run(agent.run())
    except KeyboardInterrupt:
        asyncio.run(agent.shutdown())
