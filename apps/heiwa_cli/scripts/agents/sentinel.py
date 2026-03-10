#!/usr/bin/env python3
# cli/scripts/agents/sentinel.py
"""
The Sentinel: Pull-loop daemon that bridges Plane B (Railway/Discord) to Plane A (WSL).
Watches for tasks, feeds them to the Supervisor, captures results.
"""
from __future__ import annotations

import json
import logging
import os
import sys
import time
import traceback
import psycopg2
from psycopg2.extras import RealDictCursor
from pathlib import Path
from typing import Optional, Dict, Any

# Ensure we can import from the same directory
sys.path.insert(0, str(Path(__file__).parent))

from supervisor import Supervisor

# CONFIG
DATABASE_URL = os.getenv("DATABASE_URL")
POLL_INTERVAL = 5  # Seconds

# Setup logging
def setup_logging(log_dir: Path) -> logging.Logger:
    log_dir.mkdir(parents=True, exist_ok=True)
    logger = logging.getLogger("sentinel")
    logger.setLevel(logging.INFO)
    
    formatter = logging.Formatter("%(asctime)s [%(levelname)s] SENTINEL: %(message)s")
    
    fh = logging.FileHandler(log_dir / "sentinel.log")
    fh.setFormatter(formatter)
    logger.addHandler(fh)
    
    sh = logging.StreamHandler()
    sh.setFormatter(formatter)
    logger.addHandler(sh)
    
    return logger

class PostgresQueue:
    def __init__(self, db_url: str):
        self.db_url = db_url

    def _get_conn(self):
        return psycopg2.connect(self.db_url, cursor_factory=RealDictCursor)

    def fetch_pending(self, logger: logging.Logger) -> Optional[Dict[str, Any]]:
        """
        Atomic fetch: Finds 'pending', marks 'processing', returns task.
        Uses SKIP LOCKED to allow multiple consumers if needed.
        """
        conn = None
        try:
            conn = self._get_conn()
            with conn:
                with conn.cursor() as cur:
                    cur.execute("""
                        UPDATE tasks
                        SET status = 'processing', updated_at = NOW()
                        WHERE id = (
                            SELECT id
                            FROM tasks
                            WHERE status = 'pending'
                            ORDER BY created_at ASC
                            FOR UPDATE SKIP LOCKED
                            LIMIT 1
                        )
                        RETURNING id, payload, source;
                    """)
                    return cur.fetchone()
        except Exception as e:
            logger.error(f"DB Polling Error: {e}")
            return None
        finally:
            if conn:
                conn.close()

    def complete_task(self, task_id: int, success: bool, result: str, logger: logging.Logger) -> None:
        status = 'completed' if success else 'failed'
        conn = None
        try:
            conn = self._get_conn()
            with conn:
                with conn.cursor() as cur:
                    cur.execute("""
                        UPDATE tasks 
                        SET status = %s, result = %s, updated_at = NOW()
                        WHERE id = %s
                    """, (status, result, task_id))
            logger.info(f"Task {task_id} marked as {status}.")
        except Exception as e:
            logger.error(f"Failed to update task {task_id}: {e}")
        finally:
            if conn:
                conn.close()

class Sentinel:
    def __init__(self, db_url: str, logger: logging.Logger):
        self.queue = PostgresQueue(db_url)
        self.supervisor = Supervisor() # The Brain
        self.logger = logger

    def run_forever(self) -> None:
        self.logger.info("Sentinel (PG) connected. Waiting for tasks...")
        try:
            while True:
                task = self.queue.fetch_pending(self.logger)
                if task:
                    self.execute(task)
                else:
                    time.sleep(POLL_INTERVAL)
        except KeyboardInterrupt:
            self.logger.info("Sentinel stopping...")

    def execute(self, task: Dict[str, Any]) -> None:
        t_id = task['id']
        payload = task['payload']
        self.logger.info(f"Processing Ticket #{t_id}: {payload[:50]}...")
        
        try:
            # 🧠 The Brain Logic
            result = self.supervisor.route_task(payload, complexity_score=1)
            self.queue.complete_task(t_id, success=True, result=result, logger=self.logger)
        except Exception as e:
            err = traceback.format_exc()
            self.logger.error(f"Ticket #{t_id} failed: {e}")
            self.queue.complete_task(t_id, success=False, result=err, logger=self.logger)

def main() -> int:
    root = Path(os.environ.get("HEIWA_WORKSPACE_ROOT", ".")).resolve()
    log_dir = root / "runtime" / "logs"
    logger = setup_logging(log_dir)

    if not DATABASE_URL:
        logger.critical("DATABASE_URL is missing! Export it in your shell.")
        return 1
        
    sentinel = Sentinel(DATABASE_URL, logger)
    sentinel.run_forever()
    return 0

if __name__ == "__main__":
    raise SystemExit(main())