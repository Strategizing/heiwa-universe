import asyncio
import json
import logging
import time
from typing import Dict
from swarm_hub.agents.base import BaseAgent
from protocol.protocol import Subject, Payload

logger = logging.getLogger("Spine")

class SpineAgent(BaseAgent):
    def __init__(self):
        super().__init__(name="heiwa-spine")
        # Registry: { "node_uuid": last_seen_timestamp }
        self.fleet_registry: Dict[str, float] = {}

    async def run(self):
        try:
            await self.connect()
        except Exception:
            logger.warning("⚠️ NATS unavailable. Spine running in local mode.")

        # 1. Open Ears
        await self.listen(Subject.NODE_HEARTBEAT, self.handle_heartbeat)
        
        # Re-enabling Spine routing until V2 Strategist is fully implemented
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
        Receive a Task Envelope from Messenger/API, ACK it, then dispatch planned
        steps to the executor subject(s).
        """
        payload = data.get("data", data)
        task_id = payload.get("task_id", f"task-{int(time.time())}")
        intent = payload.get("intent_class", "unknown")

        logger.info(f"📥 Received Task Envelope: {intent} (Task: {task_id})")

        # Emit an ACKNOWLEDGED status to Messenger
        ack_payload = {
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
            await self.speak(
                Subject.TASK_STATUS,
                {
                    "task_id": task_id,
                    "step_id": "spine-orchestrator",
                    "status": "BLOCKED",
                    "message": "No executable steps present in task envelope.",
                    "runtime": "spine",
                    "response_channel_id": payload.get("response_channel_id"),
                    "response_thread_id": payload.get("response_thread_id"),
                },
            )
            logger.warning("⚠️ No steps to dispatch for task %s", task_id)
            return

        if not self.nc:
            await self.speak(
                Subject.TASK_STATUS,
                {
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

        allowed_exec_subjects = {
            Subject.TASK_EXEC_REQUEST_CODE.value,
            Subject.TASK_EXEC_REQUEST_RESEARCH.value,
            Subject.TASK_EXEC_REQUEST_AUTOMATION.value,
            Subject.TASK_EXEC_REQUEST_OPERATE.value,
        }

        for step in steps:
            if not isinstance(step, dict):
                continue
            step_subject = str(step.get("subject", "")).strip()
            step_id = str(step.get("step_id", "unknown"))
            if step_subject not in allowed_exec_subjects:
                logger.warning("⚠️ Invalid exec subject for %s/%s: %s", task_id, step_id, step_subject)
                await self.speak(
                    Subject.TASK_STATUS,
                    {
                        "task_id": task_id,
                        "step_id": step_id,
                        "status": "BLOCKED",
                        "message": f"Invalid exec subject: {step_subject or 'missing'}",
                        "runtime": "spine",
                        "response_channel_id": payload.get("response_channel_id"),
                        "response_thread_id": payload.get("response_thread_id"),
                    },
                )
                continue

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
                "response_channel_id": payload.get("response_channel_id"),
                "response_thread_id": payload.get("response_thread_id"),
                "raw_text": payload.get("raw_text"),
                "normalization": payload.get("normalization"),
            }

            wrapped = {
                Payload.SENDER_ID: self.id,
                Payload.TIMESTAMP: time.time(),
                Payload.TYPE: "TASK_EXEC_DISPATCH",
                Payload.DATA: exec_payload,
            }
            await self.nc.publish(step_subject, json.dumps(wrapped).encode())
            logger.info("📤 Dispatched %s/%s to %s", task_id, step_id, step_subject)

            await self.speak(
                Subject.TASK_STATUS,
                {
                    "task_id": task_id,
                    "step_id": step_id,
                    "status": "DISPATCHED",
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

if __name__ == "__main__":
    agent = SpineAgent()
    try:
        asyncio.run(agent.run())
    except KeyboardInterrupt:
        asyncio.run(agent.shutdown())