#!/usr/bin/env python3
"""
[ANTIGRAVITY] Heiwa Muscle Node
Lightweight Remote Execution Daemon.
Connects to the Cloud HQ (Spine) to execute directed muscle tasks.
"""

import os
import sys
import time
import json
import uuid
import datetime
import subprocess
from typing import Dict, Any, List, Optional

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", ".."))
if ROOT not in sys.path:
    sys.path.insert(0, ROOT)

from libs.heiwa_sdk.heiwa_net import HeiwaNetProxy
from libs.heiwa_sdk.claw_adapter import ClawAdapter

# --- Configuration ---
HUB_URL = os.getenv("HEIWA_HUB_BASE_URL", "http://127.0.0.1:8000")
AUTH_TOKEN = os.getenv("HEIWA_AUTH_TOKEN")
NODE_ID = os.getenv("HEIWA_NODE_ID", f"muscle-{uuid.uuid4().hex[:8]}")
HEARTBEAT_INTERVAL = int(os.getenv("HEIWA_HEARTBEAT_INTERVAL", "60"))
POLL_INTERVAL = int(os.getenv("HEIWA_POLL_INTERVAL", "15"))

# Fail fast if no token
if not AUTH_TOKEN:
    print("[FATAL] HEIWA_AUTH_TOKEN not set. Identity refused.")
    sys.exit(1)

class MuscleNode:
    def __init__(self):
        self.node_id = NODE_ID
        self.instance_id = str(uuid.uuid4())
        self.hub_url = HUB_URL.rstrip("/")
        self.auth_token = AUTH_TOKEN
        self.is_running = True
        self.claw = ClawAdapter()
        self.net = HeiwaNetProxy(origin_surface="runtime", agent_id="muscle-node")
        
        print(f"[MUSCLE] Identity Locked: {self.node_id}")
        print(f"[MUSCLE] Connected to: {self.hub_url}")

    def _request(self, method: str, path: str, body: Optional[Dict] = None) -> Dict:
        """Atomic REST request with X-Auth-Token."""
        method = method.upper()
        url = f"{self.hub_url}{path}"
        data = None
        headers = {
            "X-Auth-Token": self.auth_token,
            "User-Agent": f"HeiwaMuscle/1.0 ({self.node_id})"
        }
        
        if body is not None:
            data = json.dumps(body).encode("utf-8")
            headers["Content-Type"] = "application/json"

        try:
            resp = self.net.request(
                method,
                url,
                data=data,
                headers=headers,
                timeout=30,
                purpose=f"muscle node {method} {path}",
                purpose_class="api_data_read" if method == "GET" else "api_data_write",
            )
            res_body = resp.text or ""
            if resp.status_code >= 400:
                print(f"[ERROR] Request failed [{method} {path}]: HTTP {resp.status_code} {res_body[:300]}")
                return {"error": f"http_{resp.status_code}", "detail": res_body[:1000]}
            return json.loads(res_body) if res_body else {}
        except PermissionError as e:
            print(f"[ERROR] Net policy denied [{method} {path}]: {e}")
            return {"error": str(e)}
        except Exception as e:
            print(f"[ERROR] Request failed [{method} {path}]: {e}")
            return {"error": str(e)}

    def heartbeat(self):
        """Send node heartbeat to registered identity."""
        payload = {
            "meta": {
                "os": sys.platform,
                "pid": os.getpid(),
                "instance_id": self.instance_id,
                "boot_ts": datetime.datetime.now(datetime.timezone.utc).isoformat()
            },
            "capabilities": {
                "shell": True,
                "file_ops": True
            },
            "agent_version": "1.0.0-muscle",
            "tags": ["muscle", "field-op"],
            "max_concurrency": 1
        }
        res = self._request("POST", f"/nodes/{self.node_id}/heartbeat", payload)
        if "error" not in res:
            print(f"[HEARTBEAT] {datetime.datetime.now().strftime('%H:%M:%S')} - Pulse OK")
        return res

    def poll_tasks(self):
        """Claim pending proposals for this node."""
        payload = {"node_id": self.node_id, "max_items": 1}
        res = self._request("POST", "/proposals/claim", payload)
        
        claimed = res.get("claimed", [])
        for proposal in claimed:
            self.execute_task(proposal)

    def execute_task(self, proposal: Dict):
        """Execute a claimed task and report the run."""
        proposal_id = proposal.get("proposal_id")
        payload = proposal.get("payload", {})
        command = payload.get("command")
        
        print(f"[TASK] Recieved Proposal: {proposal_id}")
        
        start_time = datetime.datetime.now(datetime.timezone.utc).isoformat()
        
        if command and command.startswith("claw:"):
            # Intelligence Task
            prompt = command[5:].strip()
            print(f"[CLAW] Intelligence injected. Solving: {prompt}")
            
            # Execute via ClawAdapter
            result = self.claw.run(prompt, agent_id="main", use_local=True) # Defaulting to local and 'main' agent
            
            # Extract status from adapter result
            if result.get("status") == "error":
                status = "error"
            elif result.get("status") == "success" or "reply" in result:
                status = "success"
            else:
                status = "partial"
                
            result["status"] = status
            
        elif command:
            # Standard Shell Task
            print(f"[EXEC] Running: {command}")
            try:
                # Execute with timeout
                process = subprocess.run(
                    command, 
                    shell=True, 
                    capture_output=True, 
                    text=True, 
                    timeout=300
                )
                
                result = {
                    "returncode": process.returncode,
                    "stdout": process.stdout,
                    "stderr": process.stderr,
                    "status": "success" if process.returncode == 0 else "failed"
                }
            except Exception as e:
                result = {
                    "status": "error",
                    "message": str(e)
                }
        else:
            print(f"[WARN] No command found in payload for {proposal_id}")
            return

        end_time = datetime.datetime.now(datetime.timezone.utc).isoformat()
        
        # Report the run
        run_payload = {
            "run_id": f"run_{uuid.uuid4().hex[:12]}",
            "proposal_id": proposal_id,
            "status": "COMPLETED" if result.get("status") == "success" else "FAILED",
            "chain_result": result,
            "node_instance_id": self.instance_id,
            "boot_ts": start_time
        }
        
        print(f"[REPORT] Run {run_payload['run_id']} finished with status: {run_payload['status']}")
        self._request("POST", "/runs", run_payload)

    def run(self):
        """Main daemon loop."""
        last_heartbeat = 0
        
        while self.is_running:
            now = time.time()
            
            # 1. Heartbeat
            if now - last_heartbeat >= HEARTBEAT_INTERVAL:
                self.heartbeat()
                last_heartbeat = now
            
            # 2. Poll Tasks
            self.poll_tasks()
            
            time.sleep(POLL_INTERVAL)

if __name__ == "__main__":
    node = MuscleNode()
    try:
        node.run()
    except KeyboardInterrupt:
        print("\n[SIGINT] Shutting down Muscle Node...")
        node.is_running = False
