# fleets/nodes/muscle/executive.py
"""
Local Executive Support: Polls for approved tasks and executes them.
Runs on Mac/Workstation only. Never on Railway.
"""
import asyncio
import os
import json
import subprocess

from libs.heiwa_sdk.nervous_system import HeiwaNervousSystem
from libs.heiwa_sdk.sanity import HeiwaSanity

async def handle_approved_task(msg):
    """Executes approved tasks locally."""
    try:
        data = json.loads(msg.data.decode())
        task_id = data.get("task_id", "unknown")
        approved_by = data.get("approved_by", "unknown")
        proposal = data.get("proposal", {})
        
        print(f"\n[EXECUTIVE] Approved Task Received: {task_id}")
        print(f"[EXECUTIVE] Approved by: {approved_by}")
        print(f"[EXECUTIVE] Proposal: {json.dumps(proposal, indent=2)}")
        
        # Extract executable content (if any)
        content = proposal.get("content", "")
        
        # Security: Scan for secrets before logging
        if not HeiwaSanity.is_safe(content):
            print("[EXECUTIVE] WARNING: Proposal contains potential secrets. Redacting.")
            content = HeiwaSanity.redact(content)
        
        # Execute if it's a bash command (ONLY local, ONLY approved)
        if proposal.get("type") == "bash":
            print(f"[EXECUTIVE] Executing: {content[:100]}...")
            result = subprocess.run(
                content, 
                shell=True, 
                capture_output=True, 
                text=True,
                timeout=300  # 5 min max
            )
            print(f"[EXECUTIVE] STDOUT: {result.stdout[:500]}")
            if result.stderr:
                print(f"[EXECUTIVE] STDERR: {result.stderr[:500]}")
        else:
            print(f"[EXECUTIVE] Non-executable proposal. Type: {proposal.get('type', 'unknown')}")
        
        await msg.ack()
        print(f"[EXECUTIVE] Task {task_id} complete.")
        
    except Exception as e:
        print(f"[EXECUTIVE] Error: {e}")

async def main():
    print("[EXECUTIVE] Local Executive Support starting...")
    nerve = HeiwaNervousSystem()
    
    while True:
        try:
            await nerve.connect()
            print("[EXECUTIVE] Connected to Nervous System.")
            
            # Subscribe to approved tasks
            await nerve.subscribe_worker("heiwa.executive.approved", handle_approved_task)
            print("[EXECUTIVE] Listening for approved tasks...")
            
            while True:
                await asyncio.sleep(1)
                
        except Exception as e:
            print(f"[EXECUTIVE] Error (Retrying in 5s): {e}")
            await asyncio.sleep(5)
        finally:
            await nerve.disconnect()

if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        print("\n[EXECUTIVE] Shutting down.")
