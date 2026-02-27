
import os

from libs.heiwa_sdk.db import Database

def main():
    print("[Cron] Starting Dead Job Requeue...")
    db = Database()
    count = db.requeue_dead_jobs(timeout_minutes=10)
    if count > 0:
        print(f"[Cron] SUCCESS: Requeued {count} dead jobs.")
    else:
        print("[Cron] No dead jobs found.")

if __name__ == "__main__":
    main()
