import json
import logging
import os
import re
import time
import uuid
from typing import Any

import discord
from discord.ext import commands

from fleets.hub.agents.base import BaseAgent
from fleets.hub.cognition.approval import ApprovalRegistry
from fleets.hub.cognition.planner import LocalTaskPlanner
from fleets.hub.protocol import Subject
from libs.heiwa_sdk.ui import UIManager
from libs.heiwa_sdk.db import Database

logger = logging.getLogger("Messenger")

STRUCTURE = {
    "🌐 HEIWA COMMAND CENTER": {
        "text": ["central-command", "local-macbook-comms", "swarm-status"],
    },
    "🛠️ DEVELOPMENT & OPS": {
        "text": ["sysops", "engineering", "deployments", "security-audit"],
    },
    "🧠 INTELLIGENCE & RESEARCH": {
        "text": ["field-intel", "research-archive", "scraper-logs"],
    },
    "📜 ARCHIVE & LOGS": {
        "text": ["task-history", "moltbook-logs"],
    }
}

class ApprovalView(discord.ui.View):
    def __init__(self, messenger: "MessengerAgent", task_id: str):
        super().__init__(timeout=messenger.approval_timeout_sec)
        self.messenger = messenger
        self.task_id = task_id
        self.prompt_message: discord.Message | None = None

    @discord.ui.button(label="Approve", style=discord.ButtonStyle.success)
    async def approve(self, interaction: discord.Interaction, _: discord.ui.Button):
        await self.messenger.apply_approval_decision(
            task_id=self.task_id,
            approved=True,
            actor=str(interaction.user),
            interaction=interaction,
        )

    @discord.ui.button(label="Reject", style=discord.ButtonStyle.danger)
    async def reject(self, interaction: discord.Interaction, _: discord.ui.Button):
        await self.messenger.apply_approval_decision(
            task_id=self.task_id,
            approved=False,
            actor=str(interaction.user),
            interaction=interaction,
        )

    async def on_timeout(self) -> None:
        for child in self.children:
            if isinstance(child, discord.ui.Button):
                child.disabled = True
        if self.prompt_message:
            try:
                await self.prompt_message.edit(view=self)
            except Exception as exc:  # pragma: no cover - Discord edit failures are non-fatal
                logger.debug("Failed to disable timed-out approval view for %s: %s", self.task_id, exc)
        await self.messenger.handle_approval_timeout(self.task_id)


class MessengerAgent(BaseAgent):
    """
    Discord control plane for Heiwa.

    Flow: ingress -> plan -> approval(optional) -> exec dispatch -> result relay.
    """

    def __init__(self):
        super().__init__(name="heiwa-messenger")
        self.db = Database()
        self.token = os.getenv("DISCORD_TOKEN") or os.getenv("DISCORD_BOT_TOKEN")
        self.conversational_mode = os.getenv("HEIWA_CONVERSATIONAL_MODE", "true").lower() == "true"
        self.selftest_allow_bot_commands = (
            os.getenv("HEIWA_SELFTEST_ALLOW_BOT_COMMANDS", "false").strip().lower() == "true"
        )
        self.listen_channel_ids = self._parse_channel_ids(os.getenv("HEIWA_LISTEN_CHANNEL_IDS", ""))
        self.intent_channel_map = self._parse_intent_channel_map(os.getenv("HEIWA_INTENT_CHANNEL_MAP", ""))

        self.approval_timeout_sec = int(os.getenv("HEIWA_APPROVAL_TIMEOUT_SEC", "600"))
        self.task_targets: dict[str, dict[str, int | None]] = {}
        self.approvals = ApprovalRegistry(timeout_sec=self.approval_timeout_sec)
        self.planner = LocalTaskPlanner()

        intents = discord.Intents.default()
        intents.message_content = True
        self.bot = commands.Bot(command_prefix="!", intents=intents)
        self.bot.event(self.on_ready)
        self.bot.event(self.on_message)

    def _get_channel_id(self, purpose: str) -> int:
        """Resolve channel ID from DB, falling back to legacy ENV if needed."""
        db_id = self.db.get_discord_channel(purpose)
        if db_id:
            return int(db_id)
        
        # Legacy fallback
        if purpose == "central-command":
            return int(os.getenv("DISCORD_CHANNEL_ID", "0"))
        return 0

    def _resolve_target_channel(self, payload: dict[str, Any], task_id: str):
        target_meta = self.task_targets.get(task_id, {})
        thread_id = payload.get("response_thread_id") or target_meta.get("thread_id")
        channel_id = payload.get("response_channel_id") or target_meta.get("channel_id")
        
        if not channel_id:
            # Resolve by intent
            intent = payload.get("intent_class", "chat")
            channel_id = self._get_channel_id(intent) or self._get_channel_id("central-command")

        target = None
        if thread_id:
            try:
                target = self.bot.get_channel(int(thread_id))
            except (TypeError, ValueError):
                target = None
        if not target and channel_id:
            try:
                target = self.bot.get_channel(int(channel_id))
            except (TypeError, ValueError):
                target = None
        return target

    async def on_ready(self):
        logger.info("🎮 Discord Connected as %s", self.bot.user)
        channel = self.bot.get_channel(self.channel_id)
        if channel:
            embed = UIManager.create_base_embed(
                "Control Plane Online", 
                "24/7 Cloud HQ established. Swarm orchestration and conversational routing active.", 
                status="online"
            )
            await channel.send(embed=embed)

    async def on_message(self, message: discord.Message):
        raw = (message.content or "").strip()
        if not raw:
            return

        if message.author == self.bot.user or message.author.bot:
            if self._allow_selftest_bot_command(message, raw):
                await self._handle_bot_control_command(message, raw)
            return

        if raw.startswith("!sync"):
            await self._sync_server_structure(message)
            return

        if raw.startswith("!dispatch"):
            instruction = raw.replace("!dispatch", "", 1).strip()
            await self._ingest_instruction(instruction, message, explicit=True)
            return

    async def _sync_server_structure(self, message: discord.Message):
        """Bootstrap or repair the Discord server structure."""
        if not message.guild:
            await message.channel.send("❌ Sync must be run within a Guild.")
            return

        if not message.author.guild_permissions.administrator:
            await message.channel.send("❌ You need Administrator permissions to sync.")
            return

        embed = UIManager.create_base_embed("Server Sync", "Architecting Heiwa Command Center...", status="thinking")
        status_msg = await message.channel.send(embed=embed)

        try:
            for cat_name, details in STRUCTURE.items():
                category = discord.utils.get(message.guild.categories, name=cat_name)
                if not category:
                    category = await message.guild.create_category(cat_name)
                
                for chan_name in details["text"]:
                    channel = discord.utils.get(category.text_channels, name=chan_name)
                    if not channel:
                        channel = await message.guild.create_text_channel(chan_name, category=category)
                    
                    # Store in DB
                    self.db.upsert_discord_channel(chan_name, channel.id, category_name=cat_name)
            
            # Special sync for central command
            self.db.upsert_discord_channel("central-command", message.channel.id, category_name="Primary")

            # Role Management
            roles_to_create = ["Heiwa Admin", "Heiwa Node", "Heiwa Researcher"]
            for rname in roles_to_create:
                role = discord.utils.get(message.guild.roles, name=rname)
                if not role:
                    role = await message.guild.create_role(name=rname, reason="Heiwa App Initialization")
                self.db.upsert_discord_role(rname, role.id)

            embed.title = "✅ Server Sync Complete"
            embed.description = "All channels architected and indexed in Sovereignty (DB)."
            embed.color = UIManager.COLORS["executing"]
            await status_msg.edit(embed=embed)
        except Exception as e:
            logger.error("Sync failed: %s", e)
            await message.channel.send(f"❌ Sync failed: {e}")

        if self.selftest_allow_bot_commands and (raw.startswith("!approve") or raw.startswith("!reject")):
            await self._handle_text_approval_command(message, raw)
            return

        if raw.startswith("!"):
            return

        if self.conversational_mode and self._should_consume_conversation(message):
            instruction = self._clean_instruction(raw)
            if instruction:
                await self._ingest_instruction(instruction, message, explicit=False)

    def _allow_selftest_bot_command(self, message: discord.Message, raw: str) -> bool:
        """Allow tightly-scoped bot/webhook command ingress for live self-tests.

        Disabled by default. When enabled, only explicit dispatch/approval commands in the
        configured control channel are accepted from bot/webhook-authored messages.
        """
        if not self.selftest_allow_bot_commands:
            return False
        if not raw.startswith(("!dispatch", "!approve", "!reject")):
            return False
        if self.channel_id and message.channel.id != self.channel_id:
            return False
        return True

    async def _handle_bot_control_command(self, message: discord.Message, raw: str) -> None:
        if raw.startswith("!dispatch"):
            instruction = raw.replace("!dispatch", "", 1).strip()
            await self._ingest_instruction(instruction, message, explicit=True)
            return
        if raw.startswith("!approve") or raw.startswith("!reject"):
            await self._handle_text_approval_command(message, raw)

    async def _handle_text_approval_command(self, message: discord.Message, raw: str) -> None:
        parts = raw.split(maxsplit=1)
        if len(parts) != 2:
            await message.channel.send("Usage: `!approve <task_id>` or `!reject <task_id>`")
            return
        cmd, task_id = parts[0].lower(), parts[1].strip()
        if not task_id:
            await message.channel.send("Missing task id.")
            return
        await self.apply_approval_decision(
            task_id=task_id,
            approved=(cmd == "!approve"),
            actor=str(message.author),
            interaction=None,
        )
        await message.channel.send(f"Recorded `{cmd[1:]}` for `{task_id}`.")

    def _should_consume_conversation(self, message: discord.Message) -> bool:
        if message.guild is None:
            return True

        if self.bot.user and self.bot.user in message.mentions:
            return True

        if message.channel.id == self.channel_id:
            return True

        return message.channel.id in self.listen_channel_ids

    def _clean_instruction(self, raw: str) -> str:
        if self.bot.user:
            raw = re.sub(rf"<@!?{self.bot.user.id}>", "", raw)
        return raw.strip()

    @staticmethod
    def _parse_channel_ids(raw: str) -> set[int]:
        out = set()
        for part in raw.split(","):
            part = part.strip()
            if not part:
                continue
            try:
                out.add(int(part))
            except ValueError:
                logger.warning("Ignoring invalid HEIWA_LISTEN_CHANNEL_IDS entry: %s", part)
        return out

    @staticmethod
    def _parse_intent_channel_map(raw: str) -> dict[str, int]:
        out: dict[str, int] = {}
        for part in raw.split(","):
            part = part.strip()
            if not part or ":" not in part:
                continue
            intent, cid = part.split(":", 1)
            try:
                out[intent.strip().lower()] = int(cid.strip())
            except ValueError:
                logger.warning("Ignoring invalid HEIWA_INTENT_CHANNEL_MAP entry: %s", part)
        return out

    @staticmethod
    def _unwrap(data: dict[str, Any]) -> dict[str, Any]:
        maybe = data.get("data")
        if isinstance(maybe, dict):
            return maybe
        return data

    async def _publish_raw(self, subject: str, data: dict[str, Any]) -> None:
        if not self.nc:
            raise RuntimeError("NATS not connected")
        await self.nc.publish(subject, json.dumps(data).encode())

    async def _ingest_instruction(self, instruction: str, message: discord.Message, explicit: bool) -> None:
        task_id = f"task-{uuid.uuid4().hex[:10]}"
        intent_profile = self.planner.normalize_intent(instruction)
        preview_intent = intent_profile.intent_class
        response_channel_id = self.intent_channel_map.get(preview_intent, message.channel.id)
        response_thread_id = message.channel.id if isinstance(message.channel, discord.Thread) else None

        # Immediate acknowledgment for chatbot UX.
        if intent_profile.intent_class != "chat":
            ack_desc = f"Heiwa acknowledged this request. Planning sequence initiated."
            if intent_profile.underspecified:
                ack_desc += "\n⚠️ *Note: Request is underspecified. I will auto-structure missing details.*"
            
            embed = UIManager.create_base_embed(
                f"Request Received",
                ack_desc,
                status="thinking"
            )
            embed.add_field(name="Task ID", value=f"`{task_id}`", inline=True)
            embed.add_field(name="Intent", value=f"`{preview_intent}`", inline=True)
            await message.channel.send(embed=embed)

            # Publish ingress event before planning for traceability.
            ingress = {
                "task_id": task_id,
                "raw_text": instruction,
                "source": "discord",
                "requested_by": str(message.author),
                "source_channel_id": message.channel.id,
                "source_message_id": message.id,
                "ingress_ts": time.time(),
                "intent_class": intent_profile.intent_class,
                "risk_level": intent_profile.risk_level,
                "requires_approval": intent_profile.requires_approval,
                "underspecified": intent_profile.underspecified,
                "normalization": intent_profile.to_dict(),
            }
            await self._publish_raw(Subject.TASK_INGRESS.value, ingress)

        # Phase 4 Optimization: Bypass planning for chat intents (routed to Flash-Lite)
        if intent_profile.intent_class == "chat":
            if self.planner.engine:
                sys_prompt = "You are Heiwa, a highly capable Cloud AI Orchestrator. Keep conversational responses very brief, friendly, and natural."
                reply = self.planner.engine.generate(prompt=instruction, runtime="railway", system=sys_prompt, complexity="low")
            else:
                reply = "Hello. (Cognitive engine unavailable)."
            reply = (reply or "").strip() or "Heiwa chat response unavailable (cognitive provider blocked/unavailable)."
            
            embed = UIManager.create_base_embed("Response", reply, status="thinking")
            await message.channel.send(embed=embed)
            return

        # Use planner to build the schema-validated task plan.
        plan = self.planner.plan(
            task_id=task_id,
            raw_text=instruction,
            requested_by=str(message.author),
            source_channel_id=message.channel.id,
            source_message_id=message.id,
            response_channel_id=response_channel_id,
            response_thread_id=response_thread_id,
            intent_profile=intent_profile,
        )
        plan_payload = plan.to_dict()
        plan_payload.setdefault("approval_id", None)

        self.task_targets[task_id] = {
            "channel_id": int(response_channel_id),
            "thread_id": int(response_thread_id) if response_thread_id else None,
        }

        await self._publish_raw(Subject.TASK_PLAN_REQUEST.value, ingress)
        await self._publish_raw(Subject.TASK_PLAN_RESULT.value, plan_payload)

        # Render visual plan
        plan_embed = UIManager.create_task_embed(
            task_id=task_id,
            instruction=instruction,
            status="thinking",
        )
        plan_embed.title = f"🧠 Heiwa Plan Ready: `{plan_payload['plan_id']}`"
        for idx, step in enumerate(plan_payload.get("steps", []), start=1):
            plan_embed.add_field(
                name=f"Step {idx}: {step['title']}",
                value=f"Runtime: `{step['target_runtime']}` | Tool: `{step['target_tool']}`",
                inline=False
            )
        await message.channel.send(embed=plan_embed)

        if plan_payload["requires_approval"]:
            approval_id = str(plan_payload.get("approval_id") or f"approval-{task_id}")
            plan_payload["approval_id"] = approval_id
            self.approvals.add(task_id, plan_payload)
            await self._publish_raw(
                Subject.TASK_APPROVAL_REQUEST.value,
                {
                    "task_id": task_id,
                    "plan_id": plan_payload["plan_id"],
                    "approval_id": approval_id,
                    "risk_level": plan_payload["risk_level"],
                    "requested_by": plan_payload["requested_by"],
                    "intent_class": plan_payload["intent_class"],
                    **self._response_target_fields(task_id, plan_payload),
                },
            )
            approval_view = ApprovalView(self, task_id)
            approval_msg = await message.channel.send(
                f"🛑 `{task_id}` requires approval (`{plan_payload['risk_level']}`).",
                view=approval_view,
            )
            approval_view.prompt_message = approval_msg
            return

        await self.dispatch_plan(plan_payload)

    @staticmethod
    def _render_plan(plan: dict[str, Any]) -> str:
        normalization = plan.get("normalization") or {}
        missing_details = normalization.get("missing_details") or []
        assumptions = normalization.get("assumptions") or []
        lines = [
            f"🧠 **Plan ready** `{plan['plan_id']}`",
            f"- intent: `{plan['intent_class']}`",
            f"- risk: `{plan['risk_level']}`",
            f"- approval: `{plan['requires_approval']}`",
            f"- underspecified: `{normalization.get('underspecified', False)}`",
            "- steps:",
        ]
        for idx, step in enumerate(plan.get("steps", []), start=1):
            lines.append(
                f"  {idx}. `{step['title']}` -> `{step['target_runtime']}/{step['target_tool']}`"
            )
        if missing_details:
            lines.append(f"- missing details: {', '.join(str(item) for item in missing_details[:3])}")
        if assumptions:
            lines.append(f"- default assumptions: {', '.join(str(item) for item in assumptions[:2])}")
        return "\n".join(lines)

    async def dispatch_plan(self, plan: dict[str, Any]) -> None:
        # V2: Publish to blackboard for capability-based routing
        await self._publish_raw(Subject.TASK_NEW.value, plan)
        logger.info(f"📤 Published Task Envelope {plan.get('task_id')} to V2 blackboard.")
        # Legacy: Keep for backward compat until Spine is fully removed
        await self._publish_raw(Subject.CORE_REQUEST.value, plan)
        logger.info(f"📤 Forwarded Task Envelope {plan.get('task_id')} to Spine Orchestrator (legacy).")

    async def handle_approval_timeout(self, task_id: str) -> None:
        state = self.approvals.expire(task_id=task_id, actor="discord", reason="approval_timeout")
        if not state or state.status != "EXPIRED":
            return

        payload = self.approvals.consume_payload(task_id)
        if not payload:
            return

        target_fields = self._response_target_fields(task_id, payload)
        await self._publish_raw(
            Subject.TASK_APPROVAL_DECISION.value,
            {
                "task_id": task_id,
                "plan_id": payload.get("plan_id"),
                "approval_id": payload.get("approval_id"),
                "approved": False,
                "actor": state.decision_by or "discord",
                "status": "EXPIRED",
                "decision_at": state.decision_at,
                "reason": state.reason or "approval_timeout",
                **target_fields,
            },
        )
        await self._publish_raw(
            Subject.TASK_STATUS.value,
            {
                "task_id": task_id,
                "plan_id": payload.get("plan_id"),
                "approval_id": payload.get("approval_id"),
                "status": "BLOCKED",
                "reason": "approval_timeout",
                "message": "Approval window expired before dispatch.",
                **target_fields,
            },
        )

    async def apply_approval_decision(
        self,
        task_id: str,
        approved: bool,
        actor: str,
        interaction: discord.Interaction | None = None,
    ) -> None:
        existing = self.approvals.get_state(task_id)
        if existing and existing.status in {"APPROVED", "REJECTED", "EXPIRED"}:
            if existing.status == "EXPIRED":
                await self.handle_approval_timeout(task_id)
            if interaction and not interaction.response.is_done():
                await interaction.response.send_message(
                    f"Task `{task_id}` is already `{existing.status}`.", ephemeral=True
                )
            return

        state = self.approvals.decide(task_id=task_id, approved=approved, actor=actor)
        if not state:
            if interaction:
                await interaction.response.send_message("Unknown task id for approval.", ephemeral=True)
            return

        payload_preview = self.approvals.get_payload(task_id) or {}
        target_fields = self._response_target_fields(task_id, payload_preview)

        await self._publish_raw(
            Subject.TASK_APPROVAL_DECISION.value,
            {
                "task_id": task_id,
                "plan_id": payload_preview.get("plan_id"),
                "approval_id": payload_preview.get("approval_id"),
                "approved": approved,
                "actor": actor,
                "status": state.status,
                "decision_at": state.decision_at,
                **target_fields,
            },
        )

        if interaction and not interaction.response.is_done():
            await interaction.response.send_message(
                f"Decision recorded for `{task_id}`: `{state.status}`", ephemeral=True
            )

        if not approved:
            await self._publish_raw(
                Subject.TASK_STATUS.value,
                {
                    "task_id": task_id,
                    "plan_id": payload_preview.get("plan_id"),
                    "approval_id": payload_preview.get("approval_id"),
                    "status": "BLOCKED",
                    "reason": "approval_rejected",
                    "message": "Task execution blocked by approval rejection.",
                    **target_fields,
                },
            )
            self.approvals.consume_payload(task_id)
            return

        payload = self.approvals.consume_payload(task_id)
        if payload:
            payload.setdefault("approval_id", payload_preview.get("approval_id"))
            await self.dispatch_plan(payload)

    async def handle_approval_decision_event(self, data: dict[str, Any]) -> None:
        payload = self._unwrap(data)
        task_id = payload.get("task_id")
        approved_raw = payload.get("approved", False)
        if isinstance(approved_raw, str):
            approved = approved_raw.strip().lower() in {"1", "true", "yes", "approve", "approved"}
        else:
            approved = bool(approved_raw)
        actor = str(payload.get("actor", "unknown"))
        if task_id:
            await self.apply_approval_decision(task_id=task_id, approved=approved, actor=actor)

    async def handle_exec_result(self, data: dict[str, Any]) -> None:
        if not self.bot.is_ready():
            return

        payload = self._unwrap(data)
        task_id = str(payload.get("task_id", "n/a"))
        status = str(payload.get("status", "PASS"))
        summary = str(payload.get("summary", "")).strip() or "(no summary)"
        
        target = self._resolve_target_channel(payload, task_id)
        if not target:
            return

        ui_status = "completed" if status == "PASS" else "error"
        embed = UIManager.create_task_embed(
            task_id, 
            instruction=summary[:100] + "...", 
            status=ui_status, 
            result=summary
        )
        await target.send(embed=embed)

    async def handle_swarm_log(self, data: dict[str, Any]):
        if not self.bot.is_ready():
            return
        payload = self._unwrap(data)
        task_id = str(payload.get("task_id", "n/a"))
        target = self._resolve_target_channel(payload, task_id)
        if not target:
            return

        sender = payload.get("agent") or data.get("sender_id", "unknown")
        result_type = payload.get("result_type", "text")
        content = str(payload.get("content", ""))
        status = payload.get("status", "INFO")
        prefix = "✅" if status == "PASS" else "⚠️"

        if result_type == "code":
            body = f"```python\n{content[:1700]}\n```"
        else:
            body = content[:1800]

        await target.send(f"{prefix} **{sender}** `{task_id}` ({result_type})\n{body}")

    async def handle_task_status(self, data: dict[str, Any]) -> None:
        if not self.bot.is_ready():
            return
        payload = self._unwrap(data)
        task_id = str(payload.get("task_id", "n/a"))
        status = str(payload.get("status", "UNKNOWN"))
        msg = str(payload.get("message", ""))

        target = self._resolve_target_channel(payload, task_id)
        if not target:
            return

        prefix = "📡"
        if status == "ACKNOWLEDGED":
            prefix = "🤝"

        lines = [
            f"{prefix} **Task Status** `{task_id}`",
            f"- status: `{status}`"
        ]
        if msg:
            lines.append(f"- info: {msg}")

        await target.send("\n".join(lines))

    async def run(self):
        if not self.token:
            logger.error("❌ No DISCORD_TOKEN found in environment")
            return

        await self.connect()

        await self.listen(Subject.TASK_STATUS, self.handle_task_status)
        await self.listen(Subject.TASK_EXEC_RESULT, self.handle_exec_result)
        await self.listen(Subject.TASK_APPROVAL_DECISION, self.handle_approval_decision_event)
        await self.listen(Subject.LOG_INFO, self.handle_swarm_log)
        await self.listen(Subject.LOG_ERROR, self.handle_swarm_log)

        logger.info("Starting Discord Gateway...")
        try:
            await self.bot.start(self.token)
        except Exception as exc:
            logger.error("Discord crash: %s", exc)
            await self.shutdown()
