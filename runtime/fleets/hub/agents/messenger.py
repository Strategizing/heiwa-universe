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
    "👑 STRATEGIC HQ": {
        "text": ["executive-briefing", "governance", "roadmap"],
        "visibility": "admin_only"
    },
    "🎮 MISSION CONTROL": {
        "text": ["operator-ingress", "central-comms", "swarm-telemetry"],
        "visibility": "admin_only"
    },
    "⚙️ ENGINEERING HUB": {
        "text": ["dev-labs", "ci-cd-stream", "infrastructure-as-code", "security-ops"],
        "visibility": "admin_only"
    },
    "🔍 RESEARCH LABS": {
        "text": ["market-intel", "technical-research", "archive-index"],
        "visibility": "admin_only"
    },
    "📡 SWARM LOGS": {
        "text": ["node-heartbeats", "audit-log", "error-trace", "thought-stream"],
        "visibility": "admin_only"
    },
    "📢 COMMUNITY SURFACE": {
        "text": ["welcome", "announcements", "open-forum"],
        "visibility": "public"
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
            except Exception as exc:
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
        
        # Priority Channel Cache
        self.channel_id = self._get_channel_id("operator-ingress") or self._get_channel_id("central-comms") or self._get_channel_id("central-command")

        self.approval_timeout_sec = int(os.getenv("HEIWA_APPROVAL_TIMEOUT_SEC", "600"))
        self.task_targets: dict[str, dict[str, int | None]] = {}
        self.approvals = ApprovalRegistry(timeout_sec=self.approval_timeout_sec)
        self.planner = LocalTaskPlanner()

        intents = discord.Intents.default()
        intents.message_content = True
        intents.members = True
        self.bot = commands.Bot(command_prefix=["!", "./", ".!"], intents=intents)
        self.bot.event(self.on_ready)
        self.bot.event(self.on_message)

    def _get_channel_id(self, purpose: str) -> int:
        """Resolve channel ID from DB, falling back to legacy ENV if needed."""
        db_id = self.db.get_discord_channel(purpose)
        if db_id:
            try:
                return int(db_id)
            except (ValueError, TypeError):
                pass
        
        # Legacy fallback
        if purpose in {"central-command", "central-comms"}:
            val = os.getenv("DISCORD_CHANNEL_ID")
            if val:
                try:
                    return int(val)
                except ValueError:
                    pass
        return 0

    def _resolve_target_channel(self, payload: dict[str, Any], task_id: str):
        target_meta = self.task_targets.get(task_id, {})
        thread_id = payload.get("response_thread_id") or target_meta.get("thread_id")
        channel_id = payload.get("response_channel_id") or target_meta.get("channel_id")
        
        if not channel_id:
            # Resolve by intent
            intent = payload.get("intent_class", "chat")
            channel_id = self._get_channel_id(intent) or self.channel_id

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
        # Attempt to post online status to ingress
        target_id = self.channel_id
        if target_id:
            channel = self.bot.get_channel(target_id)
            if channel:
                embed = UIManager.create_base_embed(
                    "Control Plane Online", 
                    "24/7 Enterprise Mesh established. Media-aware routing active.", 
                    status="online"
                )
                await channel.send(embed=embed)

    async def on_message(self, message: discord.Message):
        if message.author == self.bot.user:
            return

        if message.author.bot:
            raw = (message.content or "").strip()
            if self._allow_selftest_bot_command(message, raw):
                await self._handle_bot_control_command(message, raw)
            return

        # 1. Handle Commands
        raw = (message.content or "").strip()
        
        # Sync Command
        if any(raw.startswith(p) for p in ["!sync", "./sync", ".!sync"]):
            await self._sync_server_structure(message)
            return

        # Dispatch Command
        dispatch_prefix = next((p for p in ["!dispatch", "./dispatch", ".!dispatch"] if raw.startswith(p)), None)
        if dispatch_prefix:
            instruction = raw.replace(dispatch_prefix, "", 1).strip()
            await self._ingest_instruction(instruction, message, explicit=True)
            return

        # Approval Handlers
        if self.selftest_allow_bot_commands:
            if any(raw.startswith(p) for p in ["!approve", "./approve", ".!approve", "!reject", "./reject", ".!reject"]):
                await self._handle_text_approval_command(message, raw)
                return

        # 2. Handle Conversational Ingress
        if self.conversational_mode and self._should_consume_conversation(message):
            full_content = self._extract_full_content(message)
            if full_content:
                await self._ingest_instruction(full_content, message, explicit=False)

    def _extract_full_content(self, message: discord.Message) -> str:
        """Extract text, attachments, and embed info from a message."""
        parts = []
        if message.content:
            parts.append(self._clean_instruction(message.content))
        
        # Add Attachment Metadata
        for attachment in message.attachments:
            parts.append(f"[ATTACHMENT: {attachment.filename}]({attachment.url})")
            if attachment.content_type and attachment.content_type.startswith("image/"):
                parts.append("[MEDIA_TYPE: IMAGE]")

        # Add Embed Metadata
        for embed in message.embeds:
            embed_text = []
            if embed.title: embed_text.append(f"Title: {embed.title}")
            if embed.description: embed_text.append(f"Desc: {embed.description}")
            for field in embed.fields:
                embed_text.append(f"{field.name}: {field.value}")
            if embed_text:
                parts.append(f"[EMBED: {' | '.join(embed_text)}]")

        return "\n".join(parts).strip()

    def _should_consume_conversation(self, message: discord.Message) -> bool:
        if message.guild is None:
            return True

        if self.bot.user and self.bot.user in message.mentions:
            return True

        # Zero-Mention Command Channels
        command_purposes = {"operator-ingress", "central-comms", "operator-input", "central-command"}
        command_ids = {self._get_channel_id(p) for p in command_purposes}
        command_ids.discard(0)
        
        if message.channel.id in command_ids:
            return True

        return message.channel.id in self.listen_channel_ids

    def _clean_instruction(self, raw: str) -> str:
        if self.bot.user:
            raw = re.sub(rf"<@!?{self.bot.user.id}>", "", raw)
        return raw.strip()

    async def _sync_server_structure(self, message: discord.Message):
        """Bootstrap or repair the Discord server structure."""
        if not message.guild:
            return

        if not message.author.guild_permissions.administrator:
            await message.channel.send("❌ Permission Denied: Administrator required for structural sync.")
            return

        embed = UIManager.create_base_embed("Enterprise Architecture Sync", "Initializing High-End Structure...", status="thinking")
        status_msg = await message.channel.send(embed=embed)

        try:
            # 1. Setup Roles
            admin_role = discord.utils.get(message.guild.roles, name="Heiwa Admin")
            if not admin_role:
                admin_role = await message.guild.create_role(name="Heiwa Admin", reason="Heiwa App Initialization", permissions=discord.Permissions(administrator=True))
            self.db.upsert_discord_role("Heiwa Admin", admin_role.id)

            # 2. Build Categories and Channels
            for cat_name, details in STRUCTURE.items():
                visibility = details.get("visibility", "admin_only")
                overwrites = {
                    message.guild.default_role: discord.PermissionOverwrite(view_channel=(visibility == "public")),
                    admin_role: discord.PermissionOverwrite(view_channel=True)
                }
                
                category = discord.utils.get(message.guild.categories, name=cat_name)
                if not category:
                    category = await message.guild.create_category(cat_name, overwrites=overwrites)
                else:
                    await category.edit(overwrites=overwrites)
                
                for chan_name in details["text"]:
                    channel = discord.utils.get(category.text_channels, name=chan_name)
                    if not channel:
                        channel = await message.guild.create_text_channel(chan_name, category=category)
                    else:
                        await channel.edit(sync_permissions=True)
                    
                    # Store in DB
                    self.db.upsert_discord_channel(chan_name, channel.id, category_name=cat_name)
            
            embed.title = "✅ Swarm Structure Synchronized"
            embed.description = "Canonical enterprise structure applied and indexed."
            embed.color = UIManager.COLORS["executing"]
            await status_msg.edit(embed=embed)
            
            # Refresh local channel_id
            self.channel_id = self._get_channel_id("operator-ingress") or self._get_channel_id("central-comms")
            
        except Exception as e:
            logger.error("Sync failed: %s", e)
            await message.channel.send(f"❌ Structural Sync Failed: {e}")

    @staticmethod
    def _parse_channel_ids(raw: str) -> set[int]:
        out = set()
        if not raw: return out
        for part in raw.split(","):
            part = part.strip()
            if not part: continue
            try:
                out.add(int(part))
            except ValueError:
                pass
        return out

    @staticmethod
    def _parse_intent_channel_map(raw: str) -> dict[str, int]:
        out: dict[str, int] = {}
        if not raw: return out
        for part in raw.split(","):
            part = part.strip()
            if not part or ":" not in part: continue
            intent, cid = part.split(":", 1)
            try:
                out[intent.strip().lower()] = int(cid.strip())
            except ValueError:
                pass
        return out

    async def _publish_raw(self, subject: str, data: dict[str, Any]) -> None:
        if not self.nc:
            raise RuntimeError("NATS not connected")
        payload = {
            Payload.SENDER_ID: self.id,
            Payload.TIMESTAMP: time.time(),
            Payload.TYPE: subject,
            Payload.DATA: data
        }
        await self.nc.publish(subject, json.dumps(payload).encode())

    async def _ingest_instruction(self, instruction: str, message: discord.Message, explicit: bool) -> None:
        task_id = f"task-{uuid.uuid4().hex[:10]}"
        intent_profile = self.planner.normalize_intent(instruction)
        preview_intent = intent_profile.intent_class
        
        # Route logic
        response_channel_id = self.intent_channel_map.get(preview_intent, message.channel.id)
        response_thread_id = message.channel.id if isinstance(message.channel, discord.Thread) else None

        # Traceability metadata
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

        # Chat bypass
        if intent_profile.intent_class == "chat":
            if self.planner.engine:
                sys_prompt = "You are Heiwa, a highly capable Cloud AI Orchestrator. Keep conversational responses very brief, friendly, and natural."
                reply = self.planner.engine.generate(prompt=instruction, runtime="railway", system=sys_prompt, complexity="low")
            else:
                reply = "Hello. (Cognitive engine unavailable)."
            reply = (reply or "").strip() or "Heiwa chat response unavailable."
            
            embed = UIManager.create_base_embed("Direct Response", reply, status="online")
            await message.channel.send(embed=embed)
            return

        # Plan generation
        ack_embed = UIManager.create_base_embed("Request Received", f"Heiwa acknowledged this request. Planning sequence initiated for `{preview_intent}`.", status="thinking")
        ack_embed.add_field(name="Task ID", value=f"`{task_id}`", inline=True)
        await message.channel.send(embed=ack_embed)

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
        self.task_targets[task_id] = {
            "channel_id": int(response_channel_id),
            "thread_id": int(response_thread_id) if response_thread_id else None,
        }

        await self._publish_raw(Subject.TASK_PLAN_RESULT.value, plan_payload)

        # Visual Plan
        plan_embed = UIManager.create_task_embed(task_id, instruction, status="thinking")
        plan_embed.title = f"🧠 Heiwa Plan Ready: `{plan_payload['plan_id']}`"
        for idx, step in enumerate(plan_payload.get("steps", []), start=1):
            plan_embed.add_field(
                name=f"Step {idx}: {step['title']}",
                value=f"Runtime: `{step['target_runtime']}` | Tool: `{step['target_tool']}`",
                inline=False
            )
        await message.channel.send(embed=plan_embed)

        if plan_payload["requires_approval"]:
            approval_view = ApprovalView(self, task_id)
            approval_msg = await message.channel.send(f"🛑 `{task_id}` requires approval.", view=approval_view)
            approval_view.prompt_message = approval_msg
            self.approvals.add(task_id, plan_payload)
            return

        await self.dispatch_plan(plan_payload)

    async def dispatch_plan(self, plan: dict[str, Any]) -> None:
        await self._publish_raw(Subject.TASK_NEW.value, plan)
        await self._publish_raw(Subject.CORE_REQUEST.value, plan)

    async def handle_thought(self, data: dict[str, Any]):
        if not self.bot.is_ready(): return
        payload = self._unwrap(data)
        agent = payload.get("agent", "unknown")
        thought = payload.get("content", "")
        task_id = payload.get("task_id")
        
        channel_id = self._get_channel_id("thought-stream")
        if channel_id:
            channel = self.bot.get_channel(channel_id)
            if channel:
                embed = UIManager.create_thought_embed(agent, thought, task_id)
                await channel.send(embed=embed)

    async def handle_exec_result(self, data: dict[str, Any]) -> None:
        if not self.bot.is_ready(): return
        payload = self._unwrap(data)
        task_id = str(payload.get("task_id", "n/a"))
        status = str(payload.get("status", "PASS"))
        summary = str(payload.get("summary", "")).strip() or "(no summary)"
        usage = payload.get("usage")
        
        target = self._resolve_target_channel(payload, task_id)
        if not target: return

        embed = UIManager.create_task_embed(
            task_id, 
            instruction=summary[:100] + "...", 
            status=("completed" if status == "PASS" else "error"), 
            result=summary,
            usage=usage,
            snapshot={
                "railway": "Online",
                "node_id": payload.get("runtime", "unknown"),
                "provider": payload.get("target_tool", "OpenClaw")
            }
        )
        await target.send(embed=embed)

        # Exec briefing cross-post
        report_channel_name = payload.get("report_channel") or "executive-briefing"
        report_id = self._get_channel_id(report_channel_name)
        if report_id and report_id != target.id:
            report_chan = self.bot.get_channel(report_id)
            if report_chan: await report_chan.send(embed=embed)

    async def handle_swarm_log(self, data: dict[str, Any]):
        if not self.bot.is_ready(): return
        payload = self._unwrap(data)
        task_id = str(payload.get("task_id", "n/a"))
        target = self._resolve_target_channel(payload, task_id)
        if not target: return

        sender = payload.get("agent") or data.get("sender_id", "unknown")
        content = str(payload.get("content", ""))
        status = payload.get("status", "INFO")
        prefix = "✅" if status == "PASS" else "⚠️"
        await target.send(f"{prefix} **{sender}** `{task_id}`\n{content[:1800]}")

    async def handle_task_status(self, data: dict[str, Any]) -> None:
        if not self.bot.is_ready(): return
        payload = self._unwrap(data)
        target = self._resolve_target_channel(payload, str(payload.get("task_id", "n/a")))
        if target:
            status = str(payload.get("status", "UNKNOWN"))
            await target.send(f"📡 **Status Update** `{payload.get('task_id')}`: `{status}`")

    async def run(self):
        if not self.token:
            logger.error("❌ No DISCORD_TOKEN found")
            return
        await self.connect()
        await self.listen(Subject.TASK_STATUS, self.handle_task_status)
        await self.listen(Subject.TASK_EXEC_RESULT, self.handle_exec_result)
        await self.listen(Subject.TASK_APPROVAL_DECISION, self.handle_approval_decision_event)
        await self.listen(Subject.LOG_THOUGHT, self.handle_thought)
        await self.listen(Subject.LOG_INFO, self.handle_swarm_log)
        await self.listen(Subject.LOG_ERROR, self.handle_swarm_log)
        logger.info("Starting Discord Gateway...")
        try:
            await self.bot.start(self.token)
        except Exception as exc:
            logger.error("Discord crash: %s", exc)
            await self.shutdown()
