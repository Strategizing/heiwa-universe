import json
import logging
import subprocess
from typing import Any, List, Optional

logger = logging.getLogger("SDK.SpacetimeDB")

class SpacetimeDB:
    """
    SOTA SpacetimeDB Bridge.
    Uses the CLI as a reliable transport for calling reducers and querying state.
    """
    def __init__(self, db_identity: str, server: str = "maincloud"):
        self.db_identity = db_identity
        self.server = server

    def call(self, reducer_name: str, *args: Any) -> bool:
        """Call a reducer on the remote SpacetimeDB module."""
        # Convert args to STDB-compatible format (JSON strings if necessary)
        cmd_args = ["spacetime", "call", "--server", self.server, self.db_identity, reducer_name]
        for arg in args:
            if isinstance(arg, (dict, list)):
                cmd_args.append(json.dumps(arg))
            else:
                cmd_args.append(str(arg))

        try:
            result = subprocess.run(cmd_args, capture_output=True, text=True, timeout=10)
            if result.returncode == 0:
                logger.debug("STDB Call %s succeeded: %s", reducer_name, result.stdout.strip())
                return True
            else:
                logger.error("STDB Call %s failed: %s", reducer_name, result.stderr.strip())
                return False
        except Exception as e:
            logger.error("STDB Bridge error: %s", e)
            return False

    def query(self, sql: str) -> List[dict]:
        """Execute a SQL query against SpacetimeDB."""
        cmd = ["spacetime", "sql", "--server", self.server, "--json", self.db_identity, sql]
        try:
            result = subprocess.run(cmd, capture_output=True, text=True, timeout=10)
            if result.returncode == 0:
                try:
                    return json.loads(result.stdout)
                except json.JSONDecodeError:
                    return []
            else:
                logger.error("STDB Query failed: %s", result.stderr.strip())
                return []
        except Exception as e:
            logger.error("STDB Query error: %s", e)
            return []

    def get_user_trust(self, user_id: int) -> float:
        """Helper to fetch trust score for a Discord user."""
        res = self.query(f"SELECT trust_score FROM discord_users WHERE user_id = {user_id}")
        if res and "trust_score" in res[0]:
            return float(res[0]["trust_score"])
        return 0.5  # Default neutral trust
