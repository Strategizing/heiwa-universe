import asyncio
import os
import json
from datetime import datetime, timezone
from heiwa_hub.config import Config
from heiwa_hub.dispatch import Dispatcher
from heiwa_sdk.notifier import send_notification

class AuditWatchdog:
    """Monitors Command Audit logs and alerts on unauthorized attempts."""

    def __init__(self, bot):
        self.bot = bot
        self.last_checked_id = 0
        self.webhook_url = os.getenv("DISCORD_WEBHOOK_URL")

    async def check_audits(self):
        """Polls the DB for recent COMMAND_AUDIT alerts."""
        db = Dispatcher.get_db()
        try:
            # We query the alerts table for COMMAND_AUDIT kind
            with db.get_connection() as conn:
                cursor = conn.cursor()
                cursor.execute(
                    db._sql("SELECT id, kind, proposal_id, details_json, created_at FROM alerts WHERE kind = 'COMMAND_AUDIT' AND id > ? ORDER BY id ASC"),
                    (self.last_checked_id,)
                )
                rows = cursor.fetchall()
                
                for row in rows:
                    alert_id, kind, proposal_id, details_raw, created_at = row
                    self.last_checked_id = max(self.last_checked_id, alert_id)
                    
                    details = details_raw if isinstance(details_raw, dict) else json.loads(details_raw)
                    params = details.get("params", "")
                    
                    if "(DENIED)" in params:
                        await self.alert_unauthorized(details)
        except Exception as e:
            print(f"[WATCHDOG] Error checking audits: {e}")

    async def alert_unauthorized(self, details):
        """Send a high-priority alert about the unauthorized attempt."""
        user_name = details.get("user_name", "Unknown")
        user_id = details.get("user_id", "N/A")
        params = details.get("params", "")
        
        print(f"[WATCHDOG] 🚨 UNAUTHORIZED ATTEMPT DETECTED: {user_name} ({user_id})")
        
        embed = {
            "title": "🚨 SECURITY ALERT: UNAUTHORIZED ACCESS",
            "color": 0xFF0000,  # Red
            "fields": [
                {"name": "User", "value": f"**{user_name}** (`{user_id}`)", "inline": True},
                {"name": "Action", "value": f"`{params}`", "inline": True},
                {"name": "Node", "value": f"`{details.get('node', 'Unknown')}`", "inline": True},
                {"name": "Status", "value": "⛔ **ACCESS BLOCKED**", "inline": False},
            ],
            "footer": {"text": "[ANTIGRAVITY] Heiwa Security Watchdog"},
            "timestamp": datetime.now(timezone.utc).isoformat()
        }
        
        if self.webhook_url:
            send_notification(self.webhook_url, {"embeds": [embed]})
        else:
            print("[WATCHDOG] [WARNING] No Discord webhook configured for security alerts.")

    async def run_loop(self):
        """Main execution loop for the watchdog."""
        print("[WATCHDOG] Security Watchdog active and monitoring audits...")
        while True:
            await self.check_audits()
            await asyncio.sleep(60) # Check every minute