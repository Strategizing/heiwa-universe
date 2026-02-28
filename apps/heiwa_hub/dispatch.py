import os
import json
from datetime import datetime, timezone, timedelta

from heiwa_identity.node import load_node_identity, get_tailscale_ip
from heiwa_protocol.protocol import Subject, Payload

IDENTITY = load_node_identity()
TAILSCALE_IP = get_tailscale_ip()
from heiwa_sdk.db import Database

class Dispatcher:
    """Handles the handoff between Discord commands and remote execution targets."""
    
    _db = None

    @classmethod
    def get_db(cls):
        if cls._db is None:
            cls._db = Database()
        return cls._db

    @classmethod
    async def log_command(cls, user_id: int, user_name: str, command: str, params: str = None):
        """Record an audit trail of the command in Postgres/SQLite."""
        db = cls.get_db()
        print(f"[DISPATCH] Logging command: {command} by {user_name}")
        
        # We use a custom alert/log entry for now as a generic 'audit' table
        # might not exist in the current schema. Using record_tick or creating a 
        # dedicated 'audit_logs' table would be better. 
        # For Phase 2, we will use create_alert with a custom kind 'COMMAND_AUDIT'.
        try:
            with db.get_connection() as conn:
                cursor = conn.cursor()
                details = {
                    "user_id": user_id,
                    "user_name": user_name,
                    "params": params,
                    "node": IDENTITY.get("name"),
                    "ip": TAILSCALE_IP
                }
                db.create_alert(
                    cursor, 
                    kind="COMMAND_AUDIT", 
                    proposal_id=f"cmd_{int(datetime.now(timezone.utc).timestamp())}", 
                    node_id=IDENTITY.get("name"),
                    details=details
                )
                conn.commit()
        except Exception as e:
            print(f"[ERROR] Failed to log command to DB: {e}")

    @staticmethod
    async def run_openclaw(task: str, user_id: int, user_name: str, context: str = ""):
        """
        Dispatches a task to the Muscle nodes via the Hub DB.
        """
        await Dispatcher.log_command(user_id, user_name, "deploy", params=f"service={task}, context={context}")
        
        db = Dispatcher.get_db()
        print(f"[DISPATCH] Finding available Muscle nodes for task: {task}")
        
        try:
            # 1. Find Online Nodes and sort by freshest heartbeat
            nodes = db.list_nodes(status="ONLINE")
            if not nodes:
                return {
                    "status": "error",
                    "message": "No online Muscle nodes available to handle the deployment."
                }
            
            # Sort by last_heartbeat_at descending
            nodes.sort(key=lambda x: x.get("last_heartbeat_at", ""), reverse=True)
            
            # 2. Select the freshest node
            target_node = nodes[0]["node_id"]
            
            # 3. Create the proposal
            proposal_id = f"prop_{int(datetime.now(timezone.utc).timestamp())}_{task[:5]}"
            now = datetime.now(timezone.utc)
            expires_at = (now + timedelta(hours=1)).isoformat()
            
            proposal = {
                "proposal_id": proposal_id,
                "status": "ASSIGNED",
                "assigned_node_id": target_node,
                "hub_signature": "SIG_SPINE_AUTONOMOUS",
                "assignment_expires_at": expires_at,
                "payload": {
                    "command": f"claw: {task}"
                },
                "mode": "PRODUCTION"
            }
            
            success = db.add_proposal(proposal)
            if success:
                return {
                    "status": "success",
                    "message": f"Task '{task}' assigned to node **{target_node}** (ID: `{proposal_id}`).",
                    "node": target_node,
                    "proposal_id": proposal_id
                }
            else:
                return {"status": "error", "message": "Failed to record proposal in database."}

        except Exception as e:
            print(f"[ERROR] Dispatch failed: {e}")
            return {"status": "error", "message": str(e)}

    @staticmethod
    async def muscle_status():
        """Checks the status of registered Muscle nodes."""
        db = Dispatcher.get_db()
        nodes = db.list_nodes()
        
        # Count active nodes
        active_count = sum(1 for n in nodes if n.get("status") == "ONLINE")
        
        return {
            "spine_ip": TAILSCALE_IP,
            "status": f"{active_count} Muscle Node(s) Online",
            "db_active": True
        }