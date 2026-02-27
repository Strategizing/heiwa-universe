import asyncio
import os
from datetime import datetime, timezone

from fleets.hub.dispatch import Dispatcher
from libs.heiwa_sdk.db import Database

async def test_muscle_flow():
    print("[TEST] Initializing Muscle Flow Test...")
    
    # 1. Trigger mission from "Discord/Spine"
    print("[TEST] Triggering intelligence task: 'claw: summarize this repo'")
    result = await Dispatcher.run_openclaw("claw: summarize this repo", 12345, "test-user")
    
    if result["status"] == "success":
        print(f"[TEST] Task queued: {result['message']}")
        proposal_id = result["proposal_id"]
        
        # 2. Wait for completion (polled by muscle node in background)
        print("[TEST] Waiting for Muscle Node to pick up and COMPLETED task...")
        db = Database()
        for _ in range(30):
            await asyncio.sleep(2)
            proposal = db.get_proposal(proposal_id)
            status = proposal.get("status")
            print(f"      - Current Status: {status}")
            if status in ["CLAIMED", "COMPLETED", "FAILED"]:
                # Check for run record
                runs = db.get_runs(proposal_id=proposal_id)
                if runs:
                    print(f"✅ [TEST] Task Finished! Status: {status}")
                    print(f"      - Run ID: {runs[0]['run_id']}")
                    print(f"      - Result: {runs[0]['chain_result']}")
                    return
        print("❌ [TEST] Timeout waiting for task completion.")
    else:
        print(f"❌ [TEST] Failed to queue task: {result['message']}")

if __name__ == "__main__":
    os.environ["DATABASE_PATH"] = "./hub.db"
    asyncio.run(test_muscle_flow())
