import discord
from datetime import datetime
import os

class UIManager:
    """Advanced UI System for Heiwa Swarm with professional embeds and layout."""
    
    BRAND_NAME = "HEIWA SWARM"
    BRAND_ICON = "https://heiwa.ltd/assets/logo.png" # Placeholder if exists

    COLORS = {
        "thinking": 0x3498db,   # Blue
        "executing": 0x2ecc71,  # Green
        "overloaded": 0xf1c40f, # Yellow
        "error": 0xe74c3c,      # Red
        "online": 0x9b59b6,     # Purple
        "bridge": 0xe67e22,     # Orange
        "audit": 0x1abc9c,      # Teal
        "reasoning": 0xbdc3c7   # Silver/Gray
    }

    EMOJIS = {
        "brain": "🧠",
        "macbook": "💻",
        "cloud": "☁️",
        "warning": "⚠️",
        "success": "✅",
        "thinking": "🔵",
        "executing": "🟢",
        "bridged": "🔀",
        "lock": "🔒",
        "search": "🔍",
        "telemetry": "📡",
        "executive": "👑",
        "node": "⚙️"
    }

    @staticmethod
    def create_base_embed(title, description, status="thinking", metrics=None, snapshot=None):
        emoji = UIManager.EMOJIS.get(status, "🔵")
        embed = discord.Embed(
            title=f"{emoji} {title}",
            description=description,
            color=UIManager.COLORS.get(status, 0x95a5a6),
            timestamp=datetime.now()
        )
        embed.set_author(name=UIManager.BRAND_NAME)
        
        if metrics:
            ram = metrics.get('ram', 'N/A')
            cpu = metrics.get('cpu', 'N/A')
            embed.add_field(name=f"{UIManager.EMOJIS['node']} Node Health", value=f"RAM: `{ram}` | CPU: `{cpu}`", inline=False)
        
        # Tracked Snapshot Footer
        if snapshot:
            railway = snapshot.get("railway", "Online")
            tokens = snapshot.get("tokens", 0)
            local_health = snapshot.get("local_health", "Stable")
            node_id = snapshot.get("node_id", "Unknown")
            provider = snapshot.get("provider", "Ollama")
            
            footer_text = f"Railway: {railway} | Provider: {provider} | Tokens: {tokens} | Node: {node_id}"
            embed.set_footer(text=footer_text)
        else:
            embed.set_footer(text="Heiwa Swarm Control Plane")
            
        return embed

    @staticmethod
    def create_task_embed(task_id, instruction, status="thinking", result=None, snapshot=None, usage=None):
        emoji = UIManager.EMOJIS.get(status, "🛠️")
        embed = discord.Embed(
            title=f"{emoji} Task: `{task_id}`",
            description=f"**Instruction**: {instruction}",
            color=UIManager.COLORS.get(status, 0x3498db),
            timestamp=datetime.now()
        )
        embed.set_author(name=UIManager.BRAND_NAME)
        
        status_map = {
            "thinking": f"{UIManager.EMOJIS['brain']} Thinking...",
            "executing": f"{UIManager.EMOJIS['executing']} Executing...",
            "completed": f"{UIManager.EMOJIS['success']} Completed",
            "bridged": f"{UIManager.EMOJIS['bridged']} Bridged to Swarm",
            "error": f"{UIManager.EMOJIS['warning']} Error"
        }
        
        status_text = status_map.get(status, status)
        embed.add_field(name="Current Status", value=status_text, inline=True)
        
        if usage:
            tokens = usage.get("total_tokens", usage.get("total", 0))
            embed.add_field(name="Usage", value=f"Total Tokens: `{tokens}`", inline=True)

        if result:
            if len(result) > 1000:
                result_text = f"{result[:997]}..."
            else:
                result_text = result
            
            # Use code blocks for cleaner multi-line output
            if "## EXECUTIVE SUMMARY" in result_text:
                embed.add_field(name="Executive Brief", value=result_text, inline=False)
            else:
                embed.add_field(name="Result", value=f"```\n{result_text}\n```", inline=False)
            
        if snapshot:
            railway = snapshot.get("railway", "Online")
            node_id = snapshot.get("node_id", "Unknown")
            provider = snapshot.get("provider", "Ollama")
            footer_text = f"Cloud HQ: {railway} | Provider: {provider} | Node: {node_id}"
            embed.set_footer(text=footer_text)
            
        return embed

    @staticmethod
    def create_thought_embed(agent_name, thought, task_id=None):
        embed = discord.Embed(
            title=f"{UIManager.EMOJIS['brain']} Internal Reasoning: {agent_name}",
            description=thought,
            color=UIManager.COLORS["reasoning"],
            timestamp=datetime.now()
        )
        embed.set_author(name=UIManager.BRAND_NAME)
        if task_id:
            embed.set_footer(text=f"Context: {task_id}")
        return embed