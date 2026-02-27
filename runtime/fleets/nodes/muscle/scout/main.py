import time
import uuid
import json
import random
import os
import datetime

from libs.heiwa_sdk.db import Database

# "Mock" work to simulate a busy dev team
MOCK_PR_Snippets = [
    {
        "context": "Feature: Add User Login",
        "code": "def login(u, p): return db.query(f'SELECT * FROM users WHERE u={u}') # SQL Injection risk"
    },
    {
        "context": "Feature: Calc Tax",
        "code": "def calc_tax(amt): return amt * 0.05 # Hardcoded tax rate violation"
    },
    {
        "context": "Feature: Hello World",
        "code": "def hello(): print('Hello'); return True # Clean code"
    },
    {
        "context": "Feature: Payment Processing",
        "code": "def pay(user): stripe_key = 'sk_live_123'; return True # Critical Secret"
    }
]

def scout_loop():
    db = Database()
    print("[SCOUT] M4 Pro Scout Online. Watching for signals...")
    
    while True:
        # 1. Simulate finding work (Randomly pick a 'commit')
        work = random.choice(MOCK_PR_Snippets)
        job_id = str(uuid.uuid4())
        
        print(f"[DETECTED] New PR detected: {work['context']}")
        
        # 2. Construct Payload
        payload = {
            "type": "CODE_REVIEW",
            "code": work['code'],
            "context": work['context'],
            "source_node": "MAC_M4_SCOUT"
        }
        
        # 3. Push to Railway (The Queue)
        # Using ? for SQLite compatibility (db._exec handles translation to %s for Postgres)
        query = """
        INSERT INTO jobs (job_id, type, status, payload_json, created_at)
        VALUES (?, 'CODE_REVIEW', 'PENDING', ?, ?)
        """
        
        conn = db.get_connection()
        cursor = conn.cursor()
        try:
            now_ts = datetime.datetime.now(datetime.timezone.utc).isoformat()
            db._exec(cursor, query, (job_id, json.dumps(payload), now_ts))
            conn.commit()
            print(f"[QUEUED] Job {job_id} pushed to Railway.")
        except Exception as e:
            print(f"[ERROR] Connection to Railway lost: {e}")
        finally:
            conn.close()

        # Wait before next scan (Simulating 30s as per directive, but maybe faster for demo?)
        # Directive says "Create a job every 60 seconds (or when triggered)". Plan said 30s.
        time.sleep(30) 

if __name__ == "__main__":
    scout_loop()
