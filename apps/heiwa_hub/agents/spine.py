import asyncio
import json
import logging
import time
from typing import Dict
from heiwa_hub.agents.base import BaseAgent
from heiwa_protocol.protocol import Subject, Payload
from heiwa_hub.cognition.planner import LocalTaskPlanner

logger = logging.getLogger("Spine")

class SpineAgent(BaseAgent):
    def __init__(self):
        super().__init__(name="heiwa-spine")
        # Registry: { "node_uuid": last_seen_timestamp }
        self.fleet_registry: Dict[str, float] = {}
        self.planner = LocalTaskPlanner()

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

        # --- DIGITAL BARRIER CHECK ---
        auth_token = data.get("auth_token") or data.get("data", {}).get("auth_token")
        if not auth_token or auth_token != expected_token:
            logger.warning(f"🛡️  Digital Barrier Breach Attempt: Invalid token from sender {data.get('sender_id')}")
            return # Silent drop for security

        payload = data.get("data", data)
        task_id = payload.get("task_id", f"task-{int(time.time())}")
        
        # --- AUTO-PLANNING FOR RAW REQUESTS ---
        if not payload.get("steps") and payload.get("raw_text"):
            logger.info(f"🔮 Planning raw request for task {task_id}...")
            try:
                task_plan = self.planner.plan(
                    task_id=task_id,
                    raw_text=payload.get("raw_text"),
                    requested_by=data.get("sender_id", "unknown"),
                    source_channel_id=payload.get("source", "cli"),
                    source_message_id=task_id,
                    response_channel_id=payload.get("response_channel_id", "cli"),
                    response_thread_id=None
                )
                payload = task_plan.to_dict()
            except Exception as e:
                logger.error(f"❌ Planning failed: {e}. Falling back to direct execution.")
                # Fallback: Create a single direct step
                payload["steps"] = [{
                    "step_id": "fallback-exec",
                    "instruction": payload.get("raw_text"),
                    "subject": Subject.TASK_EXEC.value,
                    "target_runtime": "any"
                }]

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
            else:
                await self.speak(
                    Subject.TASK_STATUS,
                    {
                        "task_id": task_id,
                        "step_id": "spine-orchestrator",
                        "status": "BLOCKED",
                        "message": "No executable content found in task.",
                        "runtime": "spine"
                    },
                )
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
            # Use Subject.TASK_EXEC enum member for SOTA v3.1
            await self.speak(Subject.TASK_EXEC, exec_payload)
            logger.info("📤 Dispatched %s/%s to %s", task_id, step_id, Subject.TASK_EXEC)

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
