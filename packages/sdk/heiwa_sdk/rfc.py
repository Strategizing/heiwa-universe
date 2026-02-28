
import requests
import json
import logging
from .config import settings

try:
    from heiwa_sdk.heiwa_net import HeiwaNetProxy
    _NET_PROXY = HeiwaNetProxy(origin_surface="runtime", agent_id="rfc-client")
except ImportError:
    _NET_PROXY = None

logger = logging.getLogger(__name__)

class RFCClient:
    def __init__(self):
        self.token = settings.DISCORD_BOT_TOKEN
        self.channel_id = settings.DISCORD_CHANNEL_ID
        self.base_url = "https://discord.com/api/v10"

        if not self.token or not self.channel_id:
            logger.warning("RFC: DISCORD_BOT_TOKEN or DISCORD_CHANNEL_ID not set. RFCs will be skipped.")

    def post_rfc(self, proposal: dict) -> str | None:
        """
        Post an RFC (Request for Consent) to Discord.
        Returns the message ID if successful, or None.
        """
        if not self.token or not self.channel_id:
            return None

        # Format the content
        proposal_id = proposal.get("proposal_id", "UNKNOWN")
        payload = proposal.get("payload", {})
        if isinstance(payload, str):
            try:
                payload = json.loads(payload)
            except:
                pass

        # Extract fields
        title = payload.get("title", f"Proposal {proposal_id}")
        objective = payload.get("objective", "No objective provided")
        action_class = payload.get("action_class", "N/A")
        risk = payload.get("risk_level", "UNKNOWN")

        # Color coding
        color = 0x3498db # Blue
        if risk == "HIGH": color = 0xe74c3c # Red
        if risk == "MEDIUM": color = 0xf1c40f # Yellow

        embed = {
            "title": f"RFC: {title}",
            "description": f"**Objective**: {objective}\n**Risk**: {risk}\n**Action**: {action_class}",
            "color": color,
            "fields": [
                {"name": "ID", "value": f"`{proposal_id}`", "inline": True},
                {"name": "Fingerprint", "value": f"`{proposal.get('fingerprint', 'N/A')[:8]}...`", "inline": True}
            ],
            "footer": {"text": "Heiwa Consensus Plane"}
        }

        # Components (Buttons)
        # We need interactive components.
        # Action Row (Type 1) containing Buttons (Type 2)
        components = [
            {
                "type": 1,
                "components": [
                    {
                        "type": 2,
                        "label": "Approve",
                        "style": 3, # Green
                        "custom_id": f"consent_approve:{proposal_id}"
                        # Note: The bot listens for strict IDs usually, or we can use the persistent view ID style.
                        # `hub-bot` uses `ConsentView(proposal_id)` which generates IDs like `consent_approve` (fixed)
                        # BUT `hub-bot` code for `ConsentView` uses `custom_id="consent_approve"`.
                        # It finds the proposal ID from `self.proposal_id` which is state in the View instance.
                        # Since we are posting a *new* message from here statelessly, the Bot needs to know WHICH proposal.
                        # The `hub-bot` implementation `ConsentView` as written is tied to a specific proposal ID in memory.
                        # It does NOT appear to parse ID from custom_id.
                        # Wait, `hub-bot.py` has: `class ConsentView(discord.ui.View)` initialized with `proposal_id`.
                        # If we post a raw message with buttons, the Bot typically needs a `PersistentView` listener that parses the ID from `custom_id`.
                        # The current `hub-bot` implementation attaches the View *when sending the message*.
                        # Since `tick.py` sends the message via HTTP, `hub-bot` won't "know" about this message's view.
                        # We need to change `hub-bot` to have a Dynamic Item Listener or parse the ID from the button interactions globally.

                        # CRITICAL: `hub-bot.py` uses `ConsentView` which is attached dynamically or via `post_proposal_cmd`.
                        # If I post via HTTP, I can't attach a Python object.
                        # I must create a listener in `hub-bot` that handles these custom_ids.
                        # AND the custom_ids must encode the proposal_id: `consent_approve:<id>`.
                        # I will assume I update `hub-bot` to handle this.
                    },
                    {
                        "type": 2,
                        "label": "Reject",
                        "style": 4, # Red
                        "custom_id": f"consent_reject:{proposal_id}"
                    }
                ]
            }
        ]

        # Wait, if I change the custom_id format, I must update hub-bot.
        # Let's start with standard format.

        headers = {
            "Authorization": f"Bot {self.token}",
            "Content-Type": "application/json"
        }

        body = {
            "embeds": [embed],
            "components": components
        }

        try:
            if _NET_PROXY:
                r = _NET_PROXY.post(
                    f"{self.base_url}/channels/{self.channel_id}/messages",
                    purpose="post RFC to Discord",
                    purpose_class="messaging",
                    headers=headers, json=body,
                )
            else:
                r = requests.post(f"{self.base_url}/channels/{self.channel_id}/messages", headers=headers, json=body)
            r.raise_for_status()
            return r.json()["id"]
        except Exception as e:
            logger.error(f"Failed to post RFC: {e}")
            if r.text: logger.error(f"Discord Resp: {r.text}")
            return None

rfc_client = RFCClient()