import json
import uuid
import os

from heiwa_sdk.db import Database

# The "Toxic" Payload: Violates 'no_hardcoded_secrets' and 'no_inefficient_loops'
TOXIC_CODE = """
def process_payments(users):
    # VIOLATION 1: Hardcoded Secret
    stripe_key = "sk_live_51Mz..." 
    
    results = []
    # VIOLATION 2: Inefficient Loop (O(n^2))
    for user in users:
        for i in range(1000000): 
            pass # Busy wait
        results.append(user)
    return results
"""

def inject():
    try:
        db = Database()
        job_id = str(uuid.uuid4())
        
        print(f"💉 Injecting Poison Pill Job [{job_id}]...")
        
        # Construct the Job Payload
        payload = {
            "type": "CODE_REVIEW",
            "code": TOXIC_CODE,
            "context": "Payment processing module for production.",
            "policy_level": "STRICT"
        }

        # Insert directly into the queue (bypassing the polite API for speed)
        conn = db.get_connection()
        cursor = conn.cursor()
        try:
            query = """
            INSERT INTO jobs (job_id, type, status, payload_json, created_at)
            VALUES (?, 'CODE_REVIEW', 'PENDING', ?, ?)
            """
            import datetime
            now_ts = datetime.datetime.now(datetime.timezone.utc).isoformat()
            # Note: db._exec handles placeholder translation
            db._exec(cursor, query, (job_id, json.dumps(payload), now_ts))
            conn.commit()
            print("✅ Poison Pill Ingested. Waiting for Auditor (WSL)...")
        finally:
            conn.close()
    except Exception as e:
        print(f"❌ Injection Failed: {e}")
        # Print stack trace for debugging
        import traceback
        traceback.print_exc()

if __name__ == "__main__":
    inject()