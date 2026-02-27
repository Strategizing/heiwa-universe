import discord
from datetime import datetime

class UIManager:
    """Advanced UI System for HeiwaClaw with professional embeds and layout."""
    
    COLORS = {
        "thinking": 0x3498db,   # Blue
        "executing": 0x2ecc71,  # Green
        "overloaded": 0xf1c40f, # Yellow
        "error": 0xe74c3c,      # Red
        "online": 0x9b59b6,     # Purple
        "bridge": 0xe67e22,     # Orange
        "audit": 0x1abc9c       # Teal
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
        "search": "🔍"
    }

    @staticmethod
    def create_base_embed(title, description, status="thinking", metrics=None, snapshot=None):
        emoji = UIManager.EMOJIS.get(status, "🔵")
        embed = discord.Embed(
            title=f"{emoji} HeiwaClaw | {title}",
            description=description,
            color=UIManager.COLORS.get(status, 0x95a5a6),
            timestamp=datetime.now()
        )
        
        if metrics:
            ram = metrics.get('ram', 'N/A')
            cpu = metrics.get('cpu', 'N/A')
            scaling = metrics.get('scaling', 'OFF')
            embed.add_field(name=f"{UIManager.EMOJIS['macbook']} Local RAM", value=f"`{ram}`", inline=True)
            embed.add_field(name="⚙️ CPU", value=f"`{cpu}`", inline=True)
            embed.add_field(name=f"{UIManager.EMOJIS['brain']} Scaling", value=f"`{scaling}`", inline=True)
        
        # Tracked Snapshot Footer
        if snapshot:
            railway = snapshot.get("railway", "Online")
            tokens = snapshot.get("tokens", 0)
            local_health = snapshot.get("local_health", "Stable")
            node_id = snapshot.get("node_id", "Unknown")
            provider = snapshot.get("provider", "Ollama")
            
            footer_text = f"Railway: {railway} | Provider: {provider} | Tokens: {tokens} | Local: {local_health} | Node: {node_id}"
            embed.set_footer(text=footer_text)
        else:
            embed.set_footer(text="Heiwa Swarm Control Plane | Macbook Node")
            
        return embed

    @staticmethod
    def create_task_embed(task_id, instruction, status="thinking", result=None, snapshot=None):
        emoji = UIManager.EMOJIS.get(status, "🛠️")
        embed = discord.Embed(
            title=f"{emoji} Task: `{task_id}`",
            description=f"**Instruction**: {instruction}",
            color=UIManager.COLORS.get(status, 0x3498db),
            timestamp=datetime.now()
        )
        
        status_map = {
            "thinking": f"{UIManager.EMOJIS['brain']} Thinking...",
            "executing": f"{UIManager.EMOJIS['executing']} Executing...",
            "completed": f"{UIManager.EMOJIS['success']} Completed",
            "bridged": f"{UIManager.EMOJIS['bridged']} Bridged to Swarm",
            "error": f"{UIManager.EMOJIS['warning']} Error"
        }
        
        status_text = status_map.get(status, status)
        embed.add_field(name="Current Status", value=status_text, inline=False)
        
        if result:
            # Check if result is too long
            if len(result) > 1000:
                result_text = f"{result[:997]}..."
            else:
                result_text = result
            embed.add_field(name="Result", value=f"```\n{result_text}\n```", inline=False)
            
        if snapshot:
            railway = snapshot.get("railway", "Online")
            tokens = snapshot.get("tokens", 0)
            local_health = snapshot.get("local_health", "Stable")
            node_id = snapshot.get("node_id", "Unknown")
            provider = snapshot.get("provider", "Ollama")
            footer_text = f"Railway: {railway} | Provider: {provider} | Tokens: {tokens} | Local: {local_health} | Node: {node_id}"
            embed.set_footer(text=footer_text)
            
        return embed

    @staticmethod
    def create_system_report(metrics, snapshot):
        embed = UIManager.create_base_embed("System Health Report", "Real-time audit of local and cloud resources.", status="online", metrics=metrics, snapshot=snapshot)
        
        ollama_ram = metrics.get("ollama_ram", "N/A")
        embed.add_field(name="🧠 Ollama Footprint", value=f"`{ollama_ram}`", inline=True)
        
        return embed
