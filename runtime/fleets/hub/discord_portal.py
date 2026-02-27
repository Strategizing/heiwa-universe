# fleets/hub/discord_portal.py
"""
Heiwa Autonomous Enterprise: Discord Webhook Portal
Transforms NATS telemetry into Rich Enterprise Embeds and routes them to specific channels.
"""
import asyncio
import os
import json

from libs.heiwa_sdk.nervous_system import HeiwaNervousSystem
from libs.heiwa_sdk.heiwa_net import HeiwaAsyncNetProxy

# We now expect specific webhook URLs for specific domains, rather than one global webhook.
# If not provided, it falls back to the generic log webhook or skips.
WEBHOOKS = {
    "sysops": os.getenv("DISCORD_WEBHOOK_SYSOPS", ""),
    "engineering": os.getenv("DISCORD_WEBHOOK_ENGINEERING", ""),
    "field-intel": os.getenv("DISCORD_WEBHOOK_FIELD_INTEL", ""),
    "default": os.getenv("DISCORD_WEBHOOK_URL", "")
}

_NET_PROXY = HeiwaAsyncNetProxy(origin_surface="runtime", agent_id="discord-portal")

async def post_to_discord(embed: dict, domain: str = "default"):
    """Sends a formatted embed to the mapped Discord Webhook."""
    url = WEBHOOKS.get(domain) or WEBHOOKS.get("default")
    
    if not url:
        # Silently skip if no webhook is configured for this domain
        return
    
    payload = {"embeds": [embed]}
    try:
        resp = await _NET_PROXY.post(
            url,
            json=payload,
            timeout=10,
            purpose="discord portal routed embed",
            purpose_class="webhook_delivery",
        )
        if getattr(resp, "status", None) not in (200, 204):
            print(f"[PORTAL] Webhook post returned {getattr(resp, 'status', 'unknown')}")
    except Exception as e:
        print(f"[PORTAL] Webhook post failed: {e}")

async def handle_moltbook_log(msg):
    """Formats NATS messages into Enterprise Embeds and routes them."""
    try:
        data = json.loads(msg.data.decode())
        agent_name = data.get("agent", "Unknown Agent")
        status = data.get("status", "LOG")
        content = data.get("content", "No content.")
        
        # Determine Routing Domain based on Agent or content
        domain = "default"
        if "monitor" in agent_name.lower():
            domain = "sysops"
        elif "code" in agent_name.lower() or "github" in content.lower():
            domain = "engineering"
        elif "claw" in agent_name.lower() or "scrape" in content.lower():
            domain = "field-intel"

        # Semantic Color Coding
        color = {
            "LOG": 0x3498db,      # Blue
            "SUCCESS": 0x2ecc71,  # Green
            "ERROR": 0xe74c3c,    # Red
            "WARN": 0xf39c12,     # Orange
            "EXECUTION": 0x9b59b6 # Purple
        }.get(status, 0x95a5a6)
        
        embed = {
            "title": f"[{status}] {agent_name}",
            "description": content[:4000],  # Discord Embed Description Limit
            "color": color,
            "footer": {"text": f"Heiwa Autonomous Enterprise | Routing: #{domain}"}
        }
        
        await post_to_discord(embed, domain)
        await msg.ack()
        print(f"[PORTAL] Routed to {domain}: {agent_name} - {status}")

    except Exception as e:
        print(f"[PORTAL] Error processing message: {e}")

async def main():
    print("[PORTAL] Initializing Discord Portal...")
    nerve = HeiwaNervousSystem()
    
    while True:
        try:
            await nerve.connect()
            
            # Ensure Stream
            try:
                await nerve.js.add_stream(name="MOLTBOOK", subjects=["heiwa.moltbook.*"])
                print("[PORTAL] 'MOLTBOOK' stream ready.")
            except Exception as e:
                print(f"[PORTAL] Stream note: {e}")

            # Subscribe to logs
            await nerve.subscribe_worker("heiwa.moltbook.logs", handle_moltbook_log)
            print("[PORTAL] Listening for NATS telemetry...")
            
            while True:
                await asyncio.sleep(1)

        except Exception as e:
            print(f"[PORTAL] Critical Error (Retrying in 5s): {e}")
            await asyncio.sleep(5)
        finally:
            await nerve.disconnect()

if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("[PORTAL] Shutting down.")
