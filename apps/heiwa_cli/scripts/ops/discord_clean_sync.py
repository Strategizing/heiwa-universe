#!/usr/bin/env python3
"""
Discord Clean & Sync Utility.
Wipes old channels (except critical ones) and initializes the Heiwa structure.
"""

import asyncio
import os
import sys
from pathlib import Path

import discord

# Ensure runtime libs can be imported
MONOREPO_ROOT = Path(__file__).resolve().parents[2]
RUNTIME_ROOT = MONOREPO_ROOT / "runtime"
if str(RUNTIME_ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))
    sys.path.insert(0, str(ROOT / "packages"))
    sys.path.insert(0, str(ROOT / "apps"))

from heiwa_sdk.db import Database
from heiwa_hub.agents.messenger import STRUCTURE

class SyncClient(discord.Client):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.db = Database()

    async def on_ready(self):
        print(f"Logged in as {self.user} (ID: {self.user.id})")
        
        guild_id_str = os.getenv("DISCORD_GUILD_ID")
        if not guild_id_str:
            print("❌ DISCORD_GUILD_ID is not set in environment.")
            await self.close()
            return

        guild = self.get_guild(int(guild_id_str))
        if not guild:
            print(f"❌ Could not find guild with ID {guild_id_str}")
            await self.close()
            return

        print(f"Syncing guild: {guild.name}")

        # 1. Clean old channels not in STRUCTURE
        print("🧹 Cleaning up old channels...")
        valid_categories = set(STRUCTURE.keys())
        valid_channels = set()
        for v in STRUCTURE.values():
            valid_channels.update(v["text"])

        # Delete unrecognized channels and categories
        for channel in guild.channels:
            if isinstance(channel, discord.TextChannel):
                if channel.name not in valid_channels:
                    print(f"   Deleting text channel: {channel.name}")
                    try:
                        await channel.delete()
                    except Exception as e:
                        print(f"   ⚠️ Could not delete {channel.name}: {e}")
            elif isinstance(channel, discord.CategoryChannel):
                if channel.name not in valid_categories:
                    print(f"   Deleting category: {channel.name}")
                    try:
                        await channel.delete()
                    except Exception as e:
                        print(f"   ⚠️ Could not delete {channel.name}: {e}")

        # 1.5 Setup Roles
        print("🛡️ Setting up roles...")
        admin_role = discord.utils.get(guild.roles, name="Heiwa Admin")
        if not admin_role:
            print("   Creating role: Heiwa Admin")
            admin_role = await guild.create_role(name="Heiwa Admin", reason="Heiwa App Initialization", permissions=discord.Permissions(administrator=True))
        
        self.db.upsert_discord_role("Heiwa Admin", admin_role.id)

        # 2. Build STRUCTURE
        print("🏗️ Building Heiwa App Structure...")
        for cat_name, details in STRUCTURE.items():
            visibility = details.get("visibility", "admin_only")
            
            overwrites = {}
            if visibility == "admin_only":
                overwrites[guild.default_role] = discord.PermissionOverwrite(view_channel=False)
                overwrites[admin_role] = discord.PermissionOverwrite(view_channel=True)
            elif visibility == "public":
                overwrites[guild.default_role] = discord.PermissionOverwrite(view_channel=True)
                
            category = discord.utils.get(guild.categories, name=cat_name)
            if not category:
                print(f"   Creating category: {cat_name}")
                category = await guild.create_category(cat_name, overwrites=overwrites)
            else:
                # Update existing category permissions
                await category.edit(overwrites=overwrites)
            
            for chan_name in details["text"]:
                channel = discord.utils.get(category.text_channels, name=chan_name)
                if not channel:
                    print(f"   Creating channel: {chan_name}")
                    channel = await guild.create_text_channel(chan_name, category=category)
                else:
                    # Sync channel to category permissions
                    await channel.edit(sync_permissions=True)
                
                # Store in DB
                self.db.upsert_discord_channel(chan_name, channel.id, category_name=cat_name)
                # Primary fallback alias
                if chan_name == "central-command":
                    self.db.upsert_discord_channel("central-command", channel.id, category_name="Primary")

        print("✅ Sync complete.")
        await self.close()

def main():
    try:
        from dotenv import load_dotenv
        load_dotenv(MONOREPO_ROOT / ".env")
    except ImportError:
        print("python-dotenv not installed, assuming env is set")

    token = os.getenv("DISCORD_TOKEN") or os.getenv("DISCORD_BOT_TOKEN")
    if not token:
        print("❌ Missing DISCORD_BOT_TOKEN in environment.")
        sys.exit(1)
        
    intents = discord.Intents.default()
    client = SyncClient(intents=intents)
    client.run(token)

if __name__ == "__main__":
    main()