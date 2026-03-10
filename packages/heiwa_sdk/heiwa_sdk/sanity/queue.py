
import asyncio

from heiwa_sdk.db import Database

CONCURRENCY = 10
JOB_COUNT = 5

async def worker(worker_id):
    db = Database()
    claims = 0
    # Add small random delay to simulate network jitter?
    # Not needed for local DB lock testing usually
    
    # Try to claim jobs
    for _ in range(JOB_COUNT): 
        job = db.claim_job(f"worker-{worker_id}", job_types=["TEST_JOB"])
        if job:
            # print(f"[Worker {worker_id}] Claimed {job['job_id']}")
            claims += 1
            # Simulate work
            await asyncio.sleep(0.01)
            db.finish_job(job["job_id"], {"success": True})
        else:
           # Retry a bit
           await asyncio.sleep(0.01)
    return claims

async def main():
    print(f"--- Starting Queue Atomicity Test (Workers: {CONCURRENCY}, Jobs: {JOB_COUNT}) ---")
    db = Database()
    
    # 1. Clean old test jobs
    # Note: DELETE not exposed, but we can just create new ones
    
    # 2. Seed Jobs
    print("Seeding jobs...")
    created = []
    for i in range(JOB_COUNT):
        jid = db.create_job("TEST_JOB", {"index": i})
        created.append(jid)
    print(f"Seeded {len(created)} jobs.")

    # 3. Spawn Concurrent Workers
    print("Spawning workers...")
    tasks = [worker(i) for i in range(CONCURRENCY)]
    results = await asyncio.gather(*tasks)
    
    total_claims = sum(results)
    print(f"Total Claims by Workers: {total_claims}")
    
    # 4. Verification
    # Check DB state
    conn = db.get_connection()
    c = conn.cursor()
    
    # SQLite vs Postgres: Use raw query
    try:
        if db.use_postgres:
            c.execute("SELECT COUNT(*) FROM jobs WHERE type='TEST_JOB' AND status='DONE'")
        else:
             c.execute("SELECT COUNT(*) FROM jobs WHERE type='TEST_JOB' AND status='DONE'")
        done_count = c.fetchone()[0]
        
        print(f"DB Done Count: {done_count}")
        
        if done_count > JOB_COUNT:
             print("❌ FAIL: More jobs done than existed! (Double claims?)")
        elif total_claims > JOB_COUNT:
             print("❌ FAIL: Workers claimed more than existed! (Double claims?)")
        elif done_count == JOB_COUNT:
             print("✅ SUCCESS: All jobs handled exactly once.")
        else:
             print(f"⚠️ WARNING: Only {done_count}/{JOB_COUNT} jobs finished. (Maybe workers acted too fast?)")

    except Exception as e:
        print(f"Verification Error: {e}")
    finally:
        conn.close()

if __name__ == "__main__":
    asyncio.run(main())