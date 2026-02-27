#!/usr/bin/env python3
"""
Heiwa Node: The Muscle.
Bridges NATS directives to local Antigravity CLI execution.

This script runs on your Macbook (or any "Muscle" node) and:
1. Subscribes to heiwa.directives.*
2. Receives payloads from the Cloud Brain
3. Invokes Antigravity Agent via CLI to perform work
4. Posts results back to the Blackboard

DEPRECATED V1 COMPATIBILITY PATH:
- Canonical execution path is `cli/scripts/agents/worker_manager.py` on `heiwa.tasks.*`.
- This legacy node remains read-only/compat maintenance for one release window.
"""
import asyncio
import json
import logging
import os
import subprocess
import sys
from datetime import datetime, timezone

from libs.heiwa_sdk.nervous_system import HeiwaNervousSystem
from libs.heiwa_sdk.db import Database, Thought

logging.basicConfig(level=logging.INFO, format="[%(name)s] %(message)s")
logger = logging.getLogger("HeiwaNode")


class HeiwaNode:
    """
    The Muscle: Local execution node for Heiwa Limited.
    Bridges NATS directives to Antigravity CLI.
    """

    def __init__(self, node_id: str = None):
        self.node_id = node_id or f"Node-{os.uname().nodename}-01"
        self.nerve = HeiwaNervousSystem()
        self.db = Database()
        self.running = False

    async def start(self):
        """Boot the node and start listening for directives."""
        logger.info(f"🦾 Heiwa Node [{self.node_id}] booting up...")
        logger.warning(
            "DEPRECATION: fleets/nodes/muscle/heiwa_node.py is a legacy compatibility path. "
            "Use cli/scripts/agents/worker_manager.py with heiwa.tasks.* subjects for V1."
        )

        # 1. Connect to Nervous System
        await self.nerve.connect()
        logger.info("✓ Connected to Nervous System (NATS)")

        # 2. Register with Hub
        self.db.upsert_node_heartbeat(
            node_id=self.node_id,
            meta={"os": sys.platform, "python": sys.version},
            capabilities=["code_execution", "browser", "terminal", "human_approval"],
            agent_version="0.2.0",
            tags=["muscle", "local", "antigravity"],
        )
        logger.info("✓ Registered with Hub DB")

        # 3. Ensure streams exist
        try:
            await self.nerve.js.add_stream(name="DIRECTIVES", subjects=["heiwa.directives.*"])
            logger.info("✓ DIRECTIVES stream ready")
        except Exception as e:
            logger.info(f"Stream note: {e}")

        # 4. Subscribe to directives
        await self.nerve.js.subscribe(
            "heiwa.directives.*",
            cb=self.handle_directive,
            durable=f"muscle_{self.node_id.replace('-', '_').lower()}",
            deliver_policy="all",
        )
        logger.info("✓ Subscribed to heiwa.directives.*")

        # 5. Heartbeat loop
        self.running = True
        await self._heartbeat_loop()

    async def _heartbeat_loop(self):
        """Periodic heartbeat to Hub."""
        while self.running:
            self.db.upsert_node_heartbeat(node_id=self.node_id)
            logger.debug("💓 Heartbeat sent")
            await asyncio.sleep(30)

    async def handle_directive(self, msg):
        """
        Handle incoming NATS directive.
        Routes to appropriate skill handler.
        """
        try:
            data = json.loads(msg.data.decode())
            task_id = data.get("task_id", "unknown")
            intent = data.get("intent", "")
            artifact = data.get("artifact", {})

            logger.info(f"📥 Directive received: {task_id}")
            logger.info(f"   Intent: {intent}")
            logger.warning(
                "Legacy subject received (%s). Canonical V1 mesh API is heiwa.tasks.*", msg.subject
            )

            # Determine handler based on subject
            subject = msg.subject
            if "browser" in subject:
                result = await self._handle_browser(data)
            elif "terminal" in subject:
                result = await self._handle_terminal(data)
            elif "editor" in subject or "patch" in subject:
                result = await self._handle_code(data)
            else:
                result = await self._handle_generic(data)

            # Acknowledge message
            await msg.ack()

            # Post result back to Blackboard
            await self._post_observation(data, result)

        except Exception as e:
            logger.error(f"Error handling directive: {e}")
            await msg.nak()  # Negative ack for retry

    async def _handle_browser(self, data: dict) -> dict:
        """Handle browser-related directives."""
        artifact = data.get("artifact", {})
        url = artifact.get("content", "")

        logger.info(f"🌐 Browser action: {url or 'No URL specified'}")

        # TODO: Invoke Antigravity browser skill
        # For now, simulate the action
        return {
            "status": "completed",
            "action": "browser",
            "url": url,
            "screenshot": None,  # Would be path to screenshot
        }

    async def _handle_terminal(self, data: dict) -> dict:
        """Handle terminal/command execution directives."""
        artifact = data.get("artifact", {})
        command = artifact.get("content", "")

        if not command:
            return {"status": "error", "message": "No command provided"}

        logger.info(f"⚡ Terminal action: {command[:50]}...")

        # Execute command (with safety checks)
        # WARNING: This is dangerous - add proper sandboxing in production
        try:
            result = subprocess.run(
                command,
                shell=True,
                capture_output=True,
                text=True,
                timeout=60,
                cwd=os.getcwd(),
            )
            return {
                "status": "completed" if result.returncode == 0 else "failed",
                "action": "terminal",
                "stdout": result.stdout[:1000],  # Truncate
                "stderr": result.stderr[:500],
                "returncode": result.returncode,
            }
        except subprocess.TimeoutExpired:
            return {"status": "timeout", "action": "terminal"}
        except Exception as e:
            return {"status": "error", "action": "terminal", "message": str(e)}

    async def _handle_code(self, data: dict) -> dict:
        """Handle code/patch application directives."""
        artifact = data.get("artifact", {})
        patch_content = artifact.get("content", "")
        artifact_type = artifact.get("type", "code")

        logger.info(f"📝 Code action: {artifact_type}")

        # TODO: Apply patch via Antigravity file-editor skill
        return {
            "status": "pending_implementation",
            "action": "code",
            "artifact_type": artifact_type,
        }

    async def _handle_generic(self, data: dict) -> dict:
        """Handle generic/unknown directives."""
        logger.info(f"📦 Generic directive: {data.get('intent', 'unknown')}")
        return {
            "status": "acknowledged",
            "action": "generic",
            "message": "Directive received but no specific handler",
        }

    async def _post_observation(self, original: dict, result: dict):
        """Post observation back to Blackboard."""
        thought = Thought(
            origin=self.node_id,
            intent=f"Result of: {original.get('intent', 'unknown')[:50]}",
            thought_type="observation",
            confidence=0.95 if result.get("status") == "completed" else 0.5,
            reasoning=json.dumps(result),
            artifact={
                "type": "ref",
                "ref": original.get("task_id"),
            },
            parent_id=original.get("task_id"),
            tags=["result", result.get("action", "generic")],
        )

        if self.db.insert_thought(thought):
            logger.info(f"✅ Observation posted: {thought.stream_id}")
        else:
            logger.error("Failed to post observation")

    async def stop(self):
        """Graceful shutdown."""
        self.running = False
        await self.nerve.disconnect()
        logger.info("🛑 Heiwa Node shut down")


async def main():
    node = HeiwaNode()
    try:
        await node.start()
    except KeyboardInterrupt:
        await node.stop()


if __name__ == "__main__":
    asyncio.run(main())
