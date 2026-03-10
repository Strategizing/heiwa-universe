
import asyncio
import json

from heiwa_sdk.db import Database
# Fix import for hyphenated directory
# from fleets.nodes.muscle.auditor.main import main as auditor_main
# Since we are just testing the job submission/claiming flow using the SDK, we don't strictly need to import the Auditor main loop.
# But if we did, we'd need to use importlib.


async def test_audit_logic():
    print("--- Testing R1 Code Review Flow (Simulated) ---")
    db = Database()
    
    # 1. Create a "Bad" Code Diff
    bad_code = """
    def login(user, password):
        # TODO: Remove this
        api_key = "sk-live-1234567890" 
        print(f"Logging in {user}")
        
        while True:
            # Infinite spin
            pass
    """
    
    # 2. Submit Job
    print("Submitting CODE_REVIEW job...")
    job_id = db.create_job("CODE_REVIEW", {"diff": bad_code})
    print(f"Job IDs: {job_id}")

    # 3. Claims (Manually trigger logical claim to verify prompt construction)
    # Note: We won't run the full competitor loop because we don't have a real R1 running locally yet (mocked).
    job = db.claim_job("test-verifier", ["CODE_REVIEW"])
    if job:
        payload = job["payload"]
        diff = payload.get("diff", "")
        # Verify Policy Injection
        policy_path = Path("fleets/nodes/muscle/auditor/policy.yaml")
        if policy_path.exists():
            print("✅ Policy File Found")
        else:
             print("❌ Policy File Missing")

        if "api_key" in diff:
            print("✅ Payload correctly contains target code.")
        
        print(f"✅ Job {job['job_id']} successfully claimed and parsed.")
        
        # Cleanup
        db.finish_job(job["job_id"], {"verdict": "MOCKED_PASS"}, success=True)
    else:
        print("❌ Failed to claim job.")

if __name__ == "__main__":
    asyncio.run(test_audit_logic())