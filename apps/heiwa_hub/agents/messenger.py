import json
import logging
import os
import re
import time
import uuid
from typing import Any

import discord
from discord import app_commands
from discord.ext import commands

from heiwa_hub.agents.base import BaseAgent
from heiwa_hub.cognition.approval import ApprovalRegistry
from heiwa_hub.cognition.planner import LocalTaskPlanner
from heiwa_protocol.protocol import Subject
from heiwa_ui.manager import UIManager
from heiwa_sdk.db import Database

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
    Now optimized with Slash Commands and Media-Aware listening.
    """

    def __init__(self):
        super().__init__(name="heiwa-messenger")
        self.db = Database()
        self.token = os.getenv("DISCORD_TOKEN") or os.getenv("DISCORD_BOT_TOKEN")
        self.conversational_mode = os.getenv("HEIWA_CONVERSATIONAL_MODE", "true").lower() == "true"
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
        self.bot = commands.Bot(command_prefix="/", intents=intents)
        
        # Register App Commands
        self._register_slash_commands()

    def _register_slash_commands(self):
        @self.bot.tree.command(name="sync", description="Architect or repair the Heiwa enterprise structure")
        @app_commands.checks.has_permissions(administrator=True)
        async def sync(interaction: discord.Interaction):
            await interaction.response.defer()
            await self._run_sync(interaction)

        @self.bot.tree.command(name="ask", description="Dispatch a task to the Heiwa swarm")
        async def ask(interaction: discord.Interaction, prompt: str):
            await interaction.response.defer()
            await self._ingest_interaction(prompt, interaction, explicit=True)

        @self.bot.tree.command(name="status", description="Get real-time swarm health and node telemetry")
        async def status(interaction: discord.Interaction):
            # Future: detailed metrics from TelemetryAgent
            embed = UIManager.create_base_embed("Swarm Status", "All systems operational. Cloud HQ active.", status="online")
            await interaction.response.send_message(embed=embed)

    def _get_channel_id(self, purpose: str) -> int:
        db_id = self.db.get_discord_channel(purpose)
        if db_id:
            try: return int(db_id)
            except: pass
        if purpose in {"central-command", "central-comms"}:
            val = os.getenv("DISCORD_CHANNEL_ID")
            if val:
                try: return int(val)
                except: pass
        return 0

    def _resolve_target_channel(self, payload: dict[str, Any], task_id: str):
        target_meta = self.task_targets.get(task_id, {})
        thread_id = payload.get("response_thread_id") or target_meta.get("thread_id")
        channel_id = payload.get("response_channel_id") or target_meta.get("channel_id")
        if not channel_id:
            intent = payload.get("intent_class", "chat")
            channel_id = self._get_channel_id(intent) or self.channel_id
        target = None
        if thread_id:
            try: target = self.bot.get_channel(int(thread_id))
            except: pass
        if not target and channel_id:
            try: target = self.bot.get_channel(int(channel_id))
            except: pass
        return target

    async def on_ready(self):
        logger.info("🎮 Discord Connected as %s", self.bot.user)
        try:
            synced = await self.bot.tree.sync()
            logger.info("Synced %d slash commands.", len(synced))
        except Exception as e:
            logger.error("Failed to sync slash commands: %s", e)

        target_id = self.channel_id
        if target_id:
            channel = self.bot.get_channel(target_id)
            if channel:
                embed = UIManager.create_base_embed(
                    "Control Plane Online", 
                    "24/7 Enterprise Mesh established. Slash commands active.", 
                    status="online"
                )
                await channel.send(embed=embed)

    async def on_message(self, message: discord.Message):
        if message.author == self.bot.user or message.author.bot:
            return

        # Implicit Listener (Zero-Mention)
        if self.conversational_mode and self._should_consume_conversation(message):
            full_content = self._extract_full_content(message)
            if full_content:
                await self._ingest_interaction(full_content, message, explicit=False)

    def _extract_full_content(self, message: discord.Message) -> str:
        parts = []
        if message.content:
            parts.append(self._clean_instruction(message.content))
        for attachment in message.attachments:
            parts.append(f"[ATTACHMENT: {attachment.filename}]({attachment.url})")
            if attachment.content_type and attachment.content_type.startswith("image/"):
                parts.append("[MEDIA_TYPE: IMAGE]")
        for embed in message.embeds:
            embed_text = []
            if embed.title: embed_text.append(f"Title: {embed.title}")
            if embed.description: embed_text.append(f"Desc: {embed.description}")
            for field in embed.fields: embed_text.append(f"{field.name}: {field.value}")
            if embed_text: parts.append(f"[EMBED: {' | '.join(embed_text)}]")
        return "\n".join(parts).strip()

    def _should_consume_conversation(self, message: discord.Message) -> bool:
        if message.guild is None: return True
        if self.bot.user and self.bot.user in message.mentions: return True
        command_purposes = {"operator-ingress", "central-comms", "operator-input", "central-command"}
        command_ids = {self._get_channel_id(p) for p in command_purposes}
        command_ids.discard(0)
        return message.channel.id in command_ids

    def _clean_instruction(self, raw: str) -> str:
        if self.bot.user: raw = re.sub(rf"<@!?{self.bot.user.id}>", "", raw)
        return raw.strip()

    async def _run_sync(self, interaction: discord.Interaction):
        embed = UIManager.create_base_embed("Enterprise Architecture Sync", "Initializing High-End Structure...", status="thinking")
        msg = await interaction.followup.send(embed=embed)
        try:
            admin_role = discord.utils.get(interaction.guild.roles, name="Heiwa Admin")
            if not admin_role:
                admin_role = await interaction.guild.create_role(name="Heiwa Admin", reason="Heiwa App Initialization", permissions=discord.Permissions(administrator=True))
            self.db.upsert_discord_role("Heiwa Admin", admin_role.id)

            for cat_name, details in STRUCTURE.items():
                visibility = details.get("visibility", "admin_only")
                overwrites = {
                    interaction.guild.default_role: discord.PermissionOverwrite(view_channel=(visibility == "public")),
                    admin_role: discord.PermissionOverwrite(view_channel=True)
                }
                category = discord.utils.get(interaction.guild.categories, name=cat_name)
                if not category: category = await interaction.guild.create_category(cat_name, overwrites=overwrites)
                else: await category.edit(overwrites=overwrites)
                for chan_name in details["text"]:
                    channel = discord.utils.get(category.text_channels, name=chan_name)
                    if not channel: channel = await interaction.guild.create_text_channel(chan_name, category=category)
                    else: await channel.edit(sync_permissions=True)
                    self.db.upsert_discord_channel(chan_name, channel.id, category_name=cat_name)
            
            embed.title = "✅ Swarm Structure Synchronized"
            embed.description = "Canonical enterprise structure applied and indexed."
            embed.color = UIManager.COLORS["executing"]
            await msg.edit(embed=embed)
            self.channel_id = self._get_channel_id("operator-ingress") or self._get_channel_id("central-comms")
        except Exception as e:
            logger.error("Sync failed: %s", e)
            await interaction.followup.send(f"❌ Structural Sync Failed: {e}")

    async def _publish_raw(self, subject: str, data: dict[str, Any]) -> None:
        if not self.nc: raise RuntimeError("NATS not connected")
        payload = {
            Payload.SENDER_ID: self.id,
            Payload.TIMESTAMP: time.time(),
            Payload.TYPE: subject,
            Payload.DATA: data
        }
        await self.nc.publish(subject, json.dumps(payload).encode())

    async def _ingest_interaction(self, instruction: str, source: discord.Message | discord.Interaction, explicit: bool) -> None:
        task_id = f"task-{uuid.uuid4().hex[:10]}"
        intent_profile = self.planner.normalize_intent(instruction)
        preview_intent = intent_profile.intent_class
        
        channel = source.channel if isinstance(source, discord.Message) else source.channel
        author = source.author if isinstance(source, discord.Message) else source.user
        
        response_channel_id = self.intent_channel_map.get(preview_intent, channel.id)
        response_thread_id = channel.id if isinstance(channel, discord.Thread) else None

        ingress = {
            "task_id": task_id,
            "raw_text": instruction,
            "source": "discord",
            "requested_by": str(author),
            "source_channel_id": channel.id,
            "source_message_id": source.id if isinstance(source, discord.Message) else 0,
            "ingress_ts": time.time(),
            "intent_class": intent_profile.intent_class,
            "normalization": intent_profile.to_dict(),
        }
        await self._publish_raw(Subject.TASK_INGRESS.value, ingress)

        if intent_profile.intent_class == "chat":
            reply = self.planner.engine.generate(prompt=instruction, runtime="railway", complexity="low") if self.planner.engine else "Cognitive engine unavailable."
            embed = UIManager.create_base_embed("Direct Response", reply or "...", status="online")
            if isinstance(source, discord.Interaction): await source.followup.send(embed=embed)
            else: await source.channel.send(embed=embed)
            return

        ack_embed = UIManager.create_base_embed("Request Received", f"Planning sequence initiated for `{preview_intent}`.", status="thinking")
        ack_embed.add_field(name="Task ID", value=f"`{task_id}`", inline=True)
        if isinstance(source, discord.Interaction): await source.followup.send(embed=ack_embed)
        else: await source.channel.send(embed=ack_embed)

        plan = self.planner.plan(task_id=task_id, raw_text=instruction, requested_by=str(author), source_channel_id=channel.id, source_message_id=0, response_channel_id=response_channel_id, response_thread_id=response_thread_id, intent_profile=intent_profile)
        plan_payload = plan.to_dict()
        self.task_targets[task_id] = {"channel_id": int(response_channel_id), "thread_id": int(response_thread_id) if response_thread_id else None}
        await self._publish_raw(Subject.TASK_PLAN_RESULT.value, plan_payload)

        plan_embed = UIManager.create_task_embed(task_id, instruction, status="thinking")
        for idx, step in enumerate(plan_payload.get("steps", []), start=1):
            plan_embed.add_field(name=f"Step {idx}: {step['title']}", value=f"Runtime: `{step['target_runtime']}` | Tool: `{step['target_tool']}`", inline=False)
        
        if isinstance(source, discord.Interaction): await source.channel.send(embed=plan_embed)
        else: await source.channel.send(embed=plan_embed)

        if plan_payload["requires_approval"]:
            view = ApprovalView(self, task_id)
            msg = await source.channel.send(f"🛑 `{task_id}` requires approval.", view=view)
            view.prompt_message = msg
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
        
        # Handle decryption
        if payload.get("encrypted") and thought.startswith("[ENCRYPTED]: "):
            encrypted_val = thought.replace("[ENCRYPTED]: ", "", 1)
            thought = f"🔓 [DECRYPTED]: {self.vault.decrypt(encrypted_val)}"

        cid = self._get_channel_id("thought-stream")
        if cid:
            chan = self.bot.get_channel(cid)
            if chan: await chan.send(embed=UIManager.create_thought_embed(agent, thought, task_id))

    async def handle_telemetry(self, data: dict[str, Any]):
        """Post system metrics to the swarm-telemetry channel."""
        if not self.bot.is_ready(): return
        payload = self._unwrap(data)
        node_id = payload.get("node_id", "unknown")
        cpu = payload.get("cpu_pct", 0)
        ram = payload.get("ram_pct", 0)
        ram_used = payload.get("ram_used_gb", 0)
        ram_total = payload.get("ram_total_gb", 0)
        
        cid = self._get_channel_id("swarm-telemetry")
        if cid:
            chan = self.bot.get_channel(cid)
            if chan:
                # We use create_base_embed with metrics for a clean look
                embed = UIManager.create_base_embed(
                    f"Node Telemetry: {node_id}",
                    f"Real-time resource polling for node `{node_id}`.",
                    status="online",
                    metrics={"cpu": f"{cpu}%", "ram": f"{ram}% ({ram_used}GB / {ram_total}GB)"},
                    snapshot={"railway": "Online", "node_id": node_id, "provider": "System Poll", "tokens": 0}
                )
                await chan.send(embed=embed)

    async def handle_exec_result(self, data: dict[str, Any]) -> None:
        if not self.bot.is_ready(): return
        payload = self._unwrap(data)
        task_id = str(payload.get("task_id", "n/a"))
        target = self._resolve_target_channel(payload, task_id)
        if target:
            embed = UIManager.create_task_embed(task_id, str(payload.get("summary", ""))[:100] + "...", status=("completed" if payload.get("status") == "PASS" else "error"), result=payload.get("summary"), usage=payload.get("usage"), snapshot={"railway": "Online", "node_id": payload.get("runtime", "unknown"), "provider": payload.get("target_tool", "OpenClaw")})
            await target.send(embed=embed)
            rid = self._get_channel_id(payload.get("report_channel") or "executive-briefing")
            if rid and rid != target.id:
                rc = self.bot.get_channel(rid)
                if rc: await rc.send(embed=embed)

    async def handle_swarm_log(self, data: dict[str, Any]):
        if not self.bot.is_ready(): return
        payload = self._unwrap(data)
        target = self._resolve_target_channel(payload, str(payload.get("task_id", "n/a")))
        if target: await target.send(f"{'✅' if payload.get('status') == 'PASS' else '⚠️'} **{payload.get('agent') or 'unknown'}** `{payload.get('task_id')}`\n{str(payload.get('content', ''))[:1800]}")

    async def handle_task_status(self, data: dict[str, Any]) -> None:
        if not self.bot.is_ready(): return
        payload = self._unwrap(data)
        target = self._resolve_target_channel(payload, str(payload.get("task_id", "n/a")))
        if target: await target.send(f"📡 **Status Update** `{payload.get('task_id')}`: `{payload.get('status', 'UNKNOWN')}`")

    async def handle_task_progress(self, data: dict[str, Any]) -> None:
        if not self.bot.is_ready(): return
        payload = self._unwrap(data)
        task_id = str(payload.get("task_id", "n/a"))
        target = self._resolve_target_channel(payload, task_id)
        if target:
            content = payload.get("content", "...")
            # Use a simple message instead of a full embed for progress to avoid noise
            await target.send(f"⏳ **Task Progress** `{task_id}`: {content}")

    async def run(self):
        if not self.token: return
        await self.connect()
        await self.listen(Subject.TASK_STATUS, self.handle_task_status)
        await self.listen(Subject.TASK_PROGRESS, self.handle_task_progress)
        await self.listen(Subject.TASK_EXEC_RESULT, self.handle_exec_result)
        await self.listen(Subject.TASK_APPROVAL_DECISION, self.handle_approval_decision_event)
        await self.listen(Subject.LOG_THOUGHT, self.handle_thought)
        await self.listen(Subject.NODE_TELEMETRY, self.handle_telemetry)
        await self.listen(Subject.LOG_INFO, self.handle_swarm_log)
        await self.listen(Subject.LOG_ERROR, self.handle_swarm_log)
        logger.info("Starting Discord Gateway...")
        try: await self.bot.start(self.token)
        except Exception as exc: logger.error("Discord crash: %s", exc); await self.shutdown()