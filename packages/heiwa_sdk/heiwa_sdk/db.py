import sqlite3
import json
import datetime
import hashlib
import sys
import uuid
import os
import logging
import subprocess
import time
from pathlib import Path
from typing import Optional, List, Any, Dict, Union, Callable, TYPE_CHECKING
from .config import settings
from .spacetimedb import SpacetimeDB

# Optional Postgres support
try:
    import psycopg2
    import psycopg2.extras
    HAS_PSYCOPG2 = True
except ImportError:
    HAS_PSYCOPG2 = False


logger = logging.getLogger("SDK.Database")


class Database:
    # Type stubs for monkey-patched methods
    insert_thought: Callable[[Any], bool]
    get_stream_context: Callable[[int, int], List[Dict[str, Any]]]
    query_stream: Callable[..., List[Dict[str, Any]]]
    get_debate_chain: Callable[[str], List[Dict[str, Any]]]

    def __init__(self):
        self.state_backend = settings.HEIWA_STATE_BACKEND
        self.stdb_identity = settings.STDB_IDENTITY
        self.stdb = None
        if self.stdb_identity:
            self.stdb = SpacetimeDB(
                db_identity=self.stdb_identity,
                server=settings.STDB_SERVER,
            )
        if self.state_backend == "spacetimedb" and not self.stdb:
            raise ValueError("STDB_IDENTITY is required when HEIWA_STATE_BACKEND=spacetimedb.")
        self.use_postgres = settings.use_postgres
        self.db_path = settings.DATABASE_PATH
        self.database_url = settings.DATABASE_URL
        # No heavy IO or migrations in init - just setup config

    def init_db(self):
        """Bootstraps the database schema on startup."""
        if self.state_backend == "spacetimedb":
            logger.info("STDB backend selected; skipping compatibility SQL schema bootstrap.")
            return
        if self.use_postgres:
            logger.info("[DB] Initializing compatibility Postgres schema.")
            self._init_db_postgres()
        else:
            logger.info("[DB] Initializing compatibility SQLite schema.")
            self._ensure_persistence_check()
            self._init_db()
        
        # Shared schema: Products
        conn = self.get_connection()
        try:
            cursor = conn.cursor()
            id_def = "SERIAL PRIMARY KEY" if self.use_postgres else "INTEGER PRIMARY KEY AUTOINCREMENT"
            self._exec(cursor, f"""
                CREATE TABLE IF NOT EXISTS products (
                    id {id_def},
                    name TEXT NOT NULL,
                    description TEXT,
                    price REAL NOT NULL,
                    in_stock BOOLEAN DEFAULT TRUE
                )
            """)
            conn.commit()
            logger.info("[DB] Compatibility schema check completed.")
        except Exception as e:
            logger.error("[DB] Init failed: %s", e)
            raise e
        finally:
            conn.close()

    def _ensure_persistence_check(self):
        """Fail closed if DB path is not writable (SQLite only)."""
        path = Path(self.db_path)
        try:
            if not path.parent.exists():
                path.parent.mkdir(parents=True, exist_ok=True)
            # Try to touch the file
            path.touch(exist_ok=True)
        except Exception as e:
            logger.critical("Cannot access DATABASE_PATH %s: %s", self.db_path, e)
            sys.exit(1)

    def get_connection(self):
        if self.use_postgres:
            conn = psycopg2.connect(self.database_url)
            conn.autocommit = False
            return conn
        else:
            conn = sqlite3.connect(self.db_path)
            conn.row_factory = sqlite3.Row
            return conn

    def _row_to_dict(self, row, cursor):
        """Convert a row to dict. Works for both SQLite Row and Postgres tuple."""
        if row is None:
            return None
        if self.use_postgres:
            columns = [desc[0] for desc in cursor.description]
            return dict(zip(columns, row))
        else:
            return dict(row)

    def _rows_to_dicts(self, rows, cursor):
        """Convert multiple rows to list of dicts."""
        if self.use_postgres:
            columns = [desc[0] for desc in cursor.description]
            return [dict(zip(columns, row)) for row in rows]
        else:
            return [dict(row) for row in rows]

    @staticmethod
    def _parse_json_field(value, fallback):
        if value in (None, ""):
            return fallback
        if isinstance(value, (dict, list)):
            return value
        try:
            return json.loads(value)
        except Exception:
            return fallback

    def _targeting_from_proposal(self, proposal):
        targeting = self._parse_json_field(proposal.get("execution_targeting"), {})
        return targeting if isinstance(targeting, dict) else {}

    def _capability_set(self, node):
        caps = self._parse_json_field(node.get("capabilities_json"), {})
        if isinstance(caps, list):
            return set(str(item) for item in caps)
        if isinstance(caps, dict):
            values = set(str(key) for key, val in caps.items() if val)
            values.update(str(item) for item in caps.get("can_run", []) or [])
            values.update(str(item) for item in caps.get("models", []) or [])
            values.update(str(item) for item in caps.get("tools", []) or [])
            return values
        return set()

    def _privilege_tier_for_node(self, node):
        if node.get("privilege_tier"):
            return node.get("privilege_tier")
        meta = self._parse_json_field(node.get("meta_json"), {})
        profile = self._parse_json_field(node.get("profile_json"), {})
        return (
            meta.get("privilege_tier")
            or profile.get("privilege_tier")
            or "cloud_safe"
        )

    def _filter_eligible_nodes(self, nodes, requires, privilege_tier="cloud_safe"):
        eligible = []
        required = [str(req) for req in (requires or [])]
        for node in nodes:
            node_privilege = self._privilege_tier_for_node(node)
            if privilege_tier == "privileged_local" and node_privilege != "privileged_local":
                continue
            capability_set = self._capability_set(node)
            if all(req in capability_set for req in required):
                eligible.append(node)
        return eligible

    def _issue_stdb_capability_lease(self, proposal, holder_id, lease_expires_at, run_id=None):
        if not self.stdb:
            return None

        targeting = self._targeting_from_proposal(proposal)
        proposal_hash = proposal.get("proposal_hash")
        payload = proposal.get("payload") or ""
        if not proposal_hash:
            payload_str = payload if isinstance(payload, str) else json.dumps(payload, sort_keys=True)
            proposal_hash = hashlib.sha256(payload_str.encode("utf-8")).hexdigest()
        lease_id = f"LEASE-{uuid.uuid4().hex[:12]}"
        issued_at = datetime.datetime.now(datetime.timezone.utc).isoformat()
        lease_data = {
            "lease_id": lease_id,
            "proposal_id": proposal["proposal_id"],
            "run_id": run_id,
            "holder_kind": "node",
            "holder_id": holder_id,
            "tool_scope": targeting.get("allowed_tools") or targeting.get("tool_scope") or [],
            "network_scope": targeting.get("network_scope") or {},
            "filesystem_scope": targeting.get("filesystem_scope") or {},
            "secret_scope": targeting.get("secret_scope") or [],
            "privilege_tier": targeting.get("privilege_tier", "cloud_safe"),
            "status": "ACTIVE",
            "issued_at": issued_at,
            "expires_at": lease_expires_at,
            "hub_signature": proposal.get("hub_signature") or f"SIG-{proposal_hash[:16]}",
        }
        if not self.stdb.issue_capability_lease(lease_data):
            return None
        return lease_data

    def _sql(self, query):
        """Translate SQL placeholders from SQLite (?) to Postgres (%s) if needed."""
        if self.use_postgres:
            return query.replace("?", "%s")
        return query

    def _exec(self, cursor, query, params=None):
        """Execute SQL with placeholder translation."""
        translated = self._sql(query)
        if params:
            cursor.execute(translated, params)
        else:
            cursor.execute(translated)

    def _safe_alter_column(self, cursor, table, column, col_type):
        """Safely add a column if not exists, preventing SQL injection."""
        # Whitelist tables
        if table not in ["nodes", "proposals", "runs", "proposal_consents", "stream"]:
             raise ValueError(f"Table {table} not in whitelist")
        
        # Verify column name is alphanumeric + underscore
        if not column.replace("_", "").isalnum():
             raise ValueError(f"Invalid column name: {column}")

        try:
            if self.use_postgres:
                # Postgres 9.6+ supports ADD COLUMN IF NOT EXISTS
                cursor.execute(f"ALTER TABLE {table} ADD COLUMN IF NOT EXISTS {column} {col_type}")
            else:
                # SQLite doesn't support IF NOT EXISTS for columns
                cursor.execute(f"ALTER TABLE {table} ADD COLUMN {column} {col_type}")
        except Exception as e:
            if not self.use_postgres and "duplicate column name" in str(e).lower():
                pass
            elif "already exists" not in str(e).lower():
                print(f"[DB] Migration note for {table}.{column}: {e}")
            pass

    def _init_db_postgres(self):
        """Initialize Postgres schema."""
        conn = self.get_connection()
        cursor = conn.cursor()

        try:
            # Proposals Table
            cursor.execute(
                """
                CREATE TABLE IF NOT EXISTS proposals (
                    proposal_id TEXT PRIMARY KEY,
                    created_at TEXT NOT NULL,
                    status TEXT NOT NULL,
                    fingerprint TEXT,
                    payload TEXT NOT NULL,
                    payload_raw TEXT,
                    node_id TEXT,
                    claimed_at TEXT,
                    lease_expires_at TEXT,
                    last_heartbeat_at TEXT,
                    last_heartbeat_node_id TEXT,
                    last_heartbeat_node_instance_id TEXT,
                    last_heartbeat_detail TEXT,
                    mode TEXT
                )
            """
            )

            # Runs Table
            cursor.execute(
                """
                CREATE TABLE IF NOT EXISTS runs (
                    run_id TEXT PRIMARY KEY,
                    proposal_id TEXT NOT NULL,
                    started_at TEXT,
                    ended_at TEXT,
                    status TEXT NOT NULL,
                    chain_result TEXT,
                    signals TEXT,
                    artifact_index TEXT,
                    node_id TEXT,
                    replay_receipt TEXT,
                    mode TEXT,
                    model_id TEXT,
                    tokens_input INTEGER,
                    tokens_output INTEGER,
                    tokens_total INTEGER,
                    cost REAL
                )
            """
            )

            # Migration: Add columns to runs if missing
            for col in ["node_id", "replay_receipt", "model_id", "mode"]:
                self._safe_alter_column(cursor, "runs", col, "TEXT")
            for col in ["tokens_input", "tokens_output", "tokens_total"]:
                self._safe_alter_column(cursor, "runs", col, "INTEGER")
            self._safe_alter_column(cursor, "runs", "cost", "REAL")

            # Alerts Table
            cursor.execute(
                """
                CREATE TABLE IF NOT EXISTS alerts (
                    id TEXT PRIMARY KEY,
                    created_at TEXT NOT NULL,
                    kind TEXT NOT NULL,
                    proposal_id TEXT NOT NULL,
                    node_id TEXT,
                    claimed_at TEXT,
                    lease_expires_at TEXT,
                    ttl_seconds_remaining INTEGER,
                    status TEXT NOT NULL,
                    dedupe_key TEXT NOT NULL UNIQUE,
                    details_json TEXT
                )
            """
            )

            # Create index if not exists
            cursor.execute(
                """
                CREATE INDEX IF NOT EXISTS idx_alerts_status_created_at
                ON alerts(status, created_at DESC)
            """
            )

            # Ticks Table
            cursor.execute(
                """
                CREATE TABLE IF NOT EXISTS ticks (
                    tick_id TEXT PRIMARY KEY,
                    started_at TEXT NOT NULL,
                    ended_at TEXT,
                    status TEXT NOT NULL,
                    details_json TEXT
                )
            """
            )
            cursor.execute(
                """
                CREATE INDEX IF NOT EXISTS idx_ticks_status_ended_at
                ON ticks(status, ended_at DESC)
            """
            )

            # Liveness State (Dedupe) Table
            cursor.execute(
                """
                CREATE TABLE IF NOT EXISTS liveness_state (
                    key TEXT PRIMARY KEY,
                    last_state TEXT NOT NULL,
                    last_changed_at TEXT NOT NULL
                )
            """
            )

            # Nodes Table
            cursor.execute(
                """
                CREATE TABLE IF NOT EXISTS nodes (
                    node_id TEXT PRIMARY KEY,
                    last_heartbeat_at TEXT,
                    first_seen_at TEXT,
                    last_seen_at TEXT,
                    status TEXT,
                    meta_json TEXT,
                    capabilities_json TEXT,
                    agent_version TEXT,
                    tags_json TEXT,
                    max_concurrency INTEGER DEFAULT 1
                )
            """
            )

            # Migration: Add columns if missing
            for col in [
                "capabilities_json",
                "agent_version",
                "tags_json",
                "max_concurrency",
            ]:
                col_type = "INTEGER DEFAULT 1" if col == "max_concurrency" else "TEXT"
                self._safe_alter_column(cursor, "nodes", col, col_type)

            # Phase 2 Migration: Node Profile columns
            for col in ["trust_tier", "privilege_tier", "profile_json"]:
                self._safe_alter_column(cursor, "nodes", col, "TEXT")

            # Phase 2 Migration: Proposal Targeting & Assignment columns
            for col in [
                "execution_targeting",
                "assigned_node_id",
                "assignment_expires_at",
                "attempt_count",
                "proposal_hash",
                "hub_signature",
                "approved_at",
                "expires_at",
                "eligibility_snapshot",
            ]:
                col_type = "INTEGER DEFAULT 0" if col == "attempt_count" else "TEXT"
                self._safe_alter_column(cursor, "proposals", col, col_type)

            # Phase 2: Proposal Consents Table (Append-Only Audit Log)
            cursor.execute(
                """
                CREATE TABLE IF NOT EXISTS proposal_consents (
                    consent_id TEXT PRIMARY KEY,
                    proposal_id TEXT NOT NULL,
                    proposal_hash TEXT NOT NULL,
                    actor_type TEXT NOT NULL,
                    actor_id TEXT NOT NULL,
                    decision TEXT NOT NULL,
                    comment TEXT,
                    created_at TEXT NOT NULL
                )
            """
            )
            cursor.execute(
                """
                CREATE INDEX IF NOT EXISTS idx_consents_proposal_id
                ON proposal_consents(proposal_id)
            """
            )

            # Jobs Table (Hub-and-Spoke Core)
            cursor.execute(
                """
                CREATE TABLE IF NOT EXISTS jobs (
                    job_id TEXT PRIMARY KEY,
                    type TEXT NOT NULL,
                    status TEXT NOT NULL, -- PENDING, PROCESSING, DONE, FAILED
                    payload_json TEXT NOT NULL,
                    result_json TEXT,
                    created_at TEXT NOT NULL,
                    claimed_by_node_id TEXT,
                    claimed_at TEXT,
                    heartbeat_at TEXT,
                    lease_expires_at TEXT,
                    error_message TEXT
                )
            """
            )
            cursor.execute(
                """
                CREATE INDEX IF NOT EXISTS idx_jobs_status_created_at
                ON jobs(status, created_at ASC)
            """
            )

            # Hive Mind Stream Table (Cognitive Blackboard)
            cursor.execute(
                """
                CREATE TABLE IF NOT EXISTS stream (
                    stream_id TEXT PRIMARY KEY,
                    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
                    origin TEXT NOT NULL,
                    intent TEXT NOT NULL,
                    thought_type TEXT NOT NULL,
                    reasoning TEXT,
                    artifact JSONB,
                    confidence REAL CHECK(confidence >= 0 AND confidence <= 1),
                    parent_id TEXT,
                    tags JSONB,
                    metadata JSONB
                )
            """
            )
            cursor.execute(
                """
                CREATE INDEX IF NOT EXISTS idx_stream_origin ON stream(origin)
            """
            )
            cursor.execute(
                """
                CREATE INDEX IF NOT EXISTS idx_stream_intent ON stream(intent)
            """
            )
            cursor.execute(
                """
                CREATE INDEX IF NOT EXISTS idx_stream_timestamp ON stream(created_at DESC)
            """
            )

            # Discord Channels Table
            cursor.execute(
                """
                CREATE TABLE IF NOT EXISTS discord_channels (
                    purpose TEXT PRIMARY KEY,
                    channel_id BIGINT NOT NULL,
                    category_name TEXT,
                    permissions_json TEXT,
                    last_verified_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                )
            """
            )

            # Discord Roles Table
            cursor.execute(
                """
                CREATE TABLE IF NOT EXISTS discord_roles (
                    role_name TEXT PRIMARY KEY,
                    role_id BIGINT NOT NULL,
                    permissions_bitmask BIGINT,
                    last_verified_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
                )
            """
            )

            conn.commit()
            print("[DB] Postgres schema initialized")

        except Exception as e:
            print(f"[CRITICAL] Postgres init failed: {e}")
            conn.rollback()
            raise
        finally:
            conn.close()

    def _init_alerts(self, cursor):
        cursor.execute(
            """
            CREATE TABLE IF NOT EXISTS alerts (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL,
                kind TEXT NOT NULL,
                proposal_id TEXT NOT NULL,
                node_id TEXT,
                claimed_at TEXT,
                lease_expires_at TEXT,
                ttl_seconds_remaining INTEGER,
                status TEXT NOT NULL,
                dedupe_key TEXT NOT NULL UNIQUE,
                details_json TEXT
            )
        """
        )
        cursor.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_alerts_status_created_at
            ON alerts(status, created_at DESC)
        """
        )

    def _init_db(self):
        conn = self.get_connection()
        cursor = conn.cursor()

        # Proposals Table (with lease tracking)
        cursor.execute(
            """
            CREATE TABLE IF NOT EXISTS proposals (
                proposal_id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL,
                status TEXT NOT NULL,
                fingerprint TEXT,
                payload TEXT NOT NULL,
                payload_raw TEXT,
                node_id TEXT,
                claimed_at TEXT,
                lease_expires_at TEXT,
                last_heartbeat_at TEXT,
                last_heartbeat_node_id TEXT,
                last_heartbeat_node_instance_id TEXT,
                last_heartbeat_detail TEXT
            )
        """
        )

        # Nodes Table
        cursor.execute(
            """
            CREATE TABLE IF NOT EXISTS nodes (
                node_id TEXT PRIMARY KEY,
                last_heartbeat_at TEXT,
                first_seen_at TEXT,
                last_seen_at TEXT,
                status TEXT,
                meta_json TEXT,
                capabilities_json TEXT,
                agent_version TEXT,
                tags_json TEXT,
                max_concurrency INTEGER DEFAULT 1
            )
        """
        )

        # Migration: Add columns if missing
        for col in [
            "capabilities_json",
            "agent_version",
            "tags_json",
            "max_concurrency",
        ]:
            col_type = "INTEGER DEFAULT 1" if col == "max_concurrency" else "TEXT"
            self._safe_alter_column(cursor, "nodes", col, col_type)

        # Migration: Add columns if missing
        for col in ["payload_raw", "lease_expires_at"]:
            self._safe_alter_column(cursor, "proposals", col, "TEXT")

        for col in [
            "last_heartbeat_at",
            "last_heartbeat_node_id",
            "last_heartbeat_node_instance_id",
            "last_heartbeat_detail",
        ]:
            self._safe_alter_column(cursor, "proposals", col, "TEXT")

        # Runs Table (with node_id)
        cursor.execute(
            """
            CREATE TABLE IF NOT EXISTS runs (
                run_id TEXT PRIMARY KEY,
                proposal_id TEXT NOT NULL,
                started_at TEXT,
                ended_at TEXT,
                status TEXT NOT NULL,
                chain_result TEXT,
                signals TEXT,
                artifact_index TEXT,
                node_id TEXT,
                replay_receipt TEXT,
                FOREIGN KEY(proposal_id) REFERENCES proposals(proposal_id)
            )
        """
        )

        # Migration: Add columns if missing
        for col in ["node_id", "replay_receipt", "model_id"]:
            self._safe_alter_column(cursor, "runs", col, "TEXT")
        for col in ["tokens_input", "tokens_output", "tokens_total"]:
            self._safe_alter_column(cursor, "runs", col, "INTEGER")
        self._safe_alter_column(cursor, "runs", "cost", "REAL")

        # Migration: Add mode to proposals and runs if missing
        self._safe_alter_column(cursor, "proposals", "mode", "TEXT")
        self._safe_alter_column(cursor, "runs", "mode", "TEXT")

        # Alerts table
        self._init_alerts(cursor)

        # Ticks table
        cursor.execute(
            """
            CREATE TABLE IF NOT EXISTS ticks (
                tick_id TEXT PRIMARY KEY,
                started_at TEXT NOT NULL,
                ended_at TEXT,
                status TEXT NOT NULL,
                details_json TEXT
            )
        """
        )
        # Index for liveness checks
        cursor.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_ticks_status_ended_at
            ON ticks(status, ended_at DESC)
        """
        )

        # Liveness State (Dedupe) Table
        cursor.execute(
            """
            CREATE TABLE IF NOT EXISTS liveness_state (
                key TEXT PRIMARY KEY,
                last_state TEXT NOT NULL,
                last_changed_at TEXT NOT NULL
            )
        """
        )

        # Jobs Table (Hub-and-Spoke Core)
        cursor.execute(
            """
            CREATE TABLE IF NOT EXISTS jobs (
                job_id TEXT PRIMARY KEY,
                type TEXT NOT NULL,
                status TEXT NOT NULL, -- PENDING, PROCESSING, DONE, FAILED
                payload_json TEXT NOT NULL,
                result_json TEXT,
                created_at TEXT NOT NULL,
                claimed_by_node_id TEXT,
                claimed_at TEXT,
                heartbeat_at TEXT,
                lease_expires_at TEXT,
                error_message TEXT
            )
        """
        )
        cursor.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_jobs_status_created_at
            ON jobs(status, created_at ASC)
        """
        )

        # Hive Mind Stream Table (Cognitive Blackboard)
        cursor.execute(
            """
            CREATE TABLE IF NOT EXISTS stream (
                stream_id TEXT PRIMARY KEY,
                created_at TEXT,
                origin TEXT NOT NULL,
                intent TEXT NOT NULL,
                thought_type TEXT NOT NULL,
                reasoning TEXT,
                artifact TEXT,
                confidence REAL,
                parent_id TEXT,
                tags TEXT,
                metadata TEXT
            )
        """
        )
        cursor.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_stream_origin ON stream(origin)
        """
        )
        cursor.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_stream_intent ON stream(intent)
        """
        )
        cursor.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_stream_timestamp ON stream(created_at DESC)
        """
        )

        # Phase 2 Migration: Node Profile columns
        for col in ["trust_tier", "privilege_tier", "profile_json"]:
            self._safe_alter_column(cursor, "nodes", col, "TEXT")

        # Phase 2 Migration: Proposal Targeting & Assignment columns
        for col in [
            "execution_targeting",
            "assigned_node_id",
            "assignment_expires_at",
            "attempt_count",
            "proposal_hash",
            "hub_signature",
            "approved_at",
            "expires_at",
            "eligibility_snapshot",
        ]:
            col_type = "INTEGER DEFAULT 0" if col == "attempt_count" else "TEXT"
            self._safe_alter_column(cursor, "proposals", col, col_type)

        # Phase 2: Proposal Consents Table (Append-Only Audit Log)
        cursor.execute(
            """
            CREATE TABLE IF NOT EXISTS proposal_consents (
                consent_id TEXT PRIMARY KEY,
                proposal_id TEXT NOT NULL,
                proposal_hash TEXT NOT NULL,
                actor_type TEXT NOT NULL,
                actor_id TEXT NOT NULL,
                decision TEXT NOT NULL,
                comment TEXT,
                created_at TEXT NOT NULL
            )
        """
        )
        cursor.execute(
            """
            CREATE INDEX IF NOT EXISTS idx_consents_proposal_id
            ON proposal_consents(proposal_id)
        """
        )

        # Phase 2: Metadata column for audit
        self._safe_alter_column(cursor, "proposal_consents", "metadata", "TEXT")

        # Discord Channels
        cursor.execute(
            """
            CREATE TABLE IF NOT EXISTS discord_channels (
                purpose TEXT PRIMARY KEY,
                channel_id INTEGER NOT NULL,
                category_name TEXT,
                permissions_json TEXT,
                last_verified_at TEXT
            )
        """
        )

        # Discord Roles
        cursor.execute(
            """
            CREATE TABLE IF NOT EXISTS discord_roles (
                role_name TEXT PRIMARY KEY,
                role_id INTEGER NOT NULL,
                permissions_bitmask INTEGER,
                last_verified_at TEXT
            )
        """
        )

        conn.commit()
        conn.close()

    def get_liveness_state(self, key):
        if self.state_backend == "spacetimedb" and self.stdb:
            return self.stdb.get_liveness_state(key)
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            self._exec(cursor, "SELECT * FROM liveness_state WHERE key = ?", (key,))
            row = cursor.fetchone()
            return self._row_to_dict(row, cursor)
        finally:
            conn.close()

    def set_liveness_state(self, key, state):
        now_iso = datetime.datetime.now(datetime.timezone.utc).isoformat()
        if self.state_backend == "spacetimedb" and self.stdb:
            return self.stdb.upsert_liveness_state(key, state, now_iso)
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            # Upsert
            if self.use_postgres:
                self._exec(
                    cursor,
                    """
                    INSERT INTO liveness_state (key, last_state, last_changed_at)
                    VALUES (%s, %s, %s)
                    ON CONFLICT (key) DO UPDATE SET last_state = %s, last_changed_at = %s
                """,
                    (key, state, now_iso, state, now_iso),
                )
            else:
                self._exec(
                    cursor,
                    """
                    INSERT INTO liveness_state (key, last_state, last_changed_at)
                    VALUES (?, ?, ?)
                    ON CONFLICT(key) DO UPDATE SET last_state = ?, last_changed_at = ?
                """,
                    (key, state, now_iso, state, now_iso),
                )
            conn.commit()
            return True
        except Exception:
            conn.rollback()
            return False
        finally:
            conn.close()

    def get_liveness_state(self, key):
        """Get current liveness state for a service key."""
        if self.state_backend == "spacetimedb" and self.stdb:
            return self.stdb.get_liveness_state(key)
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            self._exec(
                cursor,
                "SELECT last_state, last_changed_at FROM liveness_state WHERE key = ?",
                (key,),
            )
            row = cursor.fetchone()
            if row:
                return self._row_to_dict(row, cursor)
            return None
        finally:
            conn.close()

    def record_tick(self, tick_data):
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            self._exec(
                cursor,
                """
                INSERT INTO ticks (tick_id, started_at, ended_at, status, details_json)
                VALUES (?, ?, ?, ?, ?)
            """,
                (
                    tick_data["tick_id"],
                    tick_data["started_at"],
                    tick_data.get("ended_at"),
                    tick_data["status"],
                    json.dumps(tick_data.get("details_json", {})),
                ),
            )
            conn.commit()
            return True
        except Exception as e:
            conn.rollback()
            print(f"DB Tick Error: {e}")
            return False
        finally:
            conn.close()

    def get_recent_ticks(self, hours=24):
        """Fetch basic stats for ticks in the last N hours."""
        conn = self.get_connection()
        cursor = conn.cursor()

        # Calculate cutoff
        cutoff = (
            datetime.datetime.now(datetime.timezone.utc)
            - datetime.timedelta(hours=hours)
        ).isoformat()

        try:
            self._exec(
                cursor,
                """
                SELECT tick_id, status, details_json
                FROM ticks 
                WHERE started_at > ?
                ORDER BY started_at ASC
            """,
                (cutoff,),
            )
            rows = cursor.fetchall()
            results = []
            for row in rows:
                r = self._row_to_dict(row, cursor)
                # Parse JSON fields if present
                if r.get("details_json"):
                    try:
                        r["details"] = json.loads(r["details_json"])
                    except Exception:
                        r["details"] = {}
                results.append(r)
            return results
        finally:
            conn.close()

    def upsert_node_heartbeat(
        self,
        node_id,
        meta=None,
        capabilities=None,
        agent_version=None,
        tags=None,
        max_concurrency=1,
    ):
        """Update node heartbeat and status."""
        if self.state_backend == "spacetimedb" and self.stdb:
            return self.stdb.upsert_node_heartbeat(
                node_id=node_id,
                meta=meta,
                capabilities=capabilities,
                agent_version=agent_version,
                tags=tags,
                max_concurrency=max_concurrency,
            )
        conn = self.get_connection()
        cursor = conn.cursor()
        now = datetime.datetime.now(datetime.timezone.utc).isoformat()

        try:
            # Check if exists
            self._exec(
                cursor, "SELECT first_seen_at FROM nodes WHERE node_id = ?", (node_id,)
            )
            row = cursor.fetchone()

            meta_json = json.dumps(meta) if meta else "{}"
            cap_json = json.dumps(capabilities) if capabilities else "{}"
            tags_json = json.dumps(tags) if tags else "[]"

            if row:
                # Update
                self._exec(
                    cursor,
                    """
                    UPDATE nodes 
                    SET last_heartbeat_at = ?, last_seen_at = ?, status = 'ONLINE', 
                        meta_json = ?, capabilities_json = ?, agent_version = ?, tags_json = ?, max_concurrency = ?
                    WHERE node_id = ?
                """,
                    (
                        now,
                        now,
                        meta_json,
                        cap_json,
                        agent_version,
                        tags_json,
                        max_concurrency,
                        node_id,
                    ),
                )
            else:
                # Insert
                self._exec(
                    cursor,
                    """
                    INSERT INTO nodes (node_id, last_heartbeat_at, first_seen_at, last_seen_at, status, meta_json, capabilities_json, agent_version, tags_json, max_concurrency)
                    VALUES (?, ?, ?, ?, 'ONLINE', ?, ?, ?, ?, ?)
                """,
                    (
                        node_id,
                        now,
                        now,
                        now,
                        meta_json,
                        cap_json,
                        agent_version,
                        tags_json,
                        max_concurrency,
                    ),
                )
            conn.commit()
            return True
        except Exception as e:
            conn.rollback()
            print(f"DB Node Heartbeat Error: {e}")
            return False
        finally:
            conn.close()

    def list_nodes(self, status=None):
        """List nodes, optionally filtered by status."""
        if self.state_backend == "spacetimedb" and self.stdb:
            return self.stdb.list_nodes(status=status)
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            if status:
                self._exec(
                    cursor,
                    "SELECT * FROM nodes WHERE status = ? ORDER BY node_id",
                    (status,),
                )
            else:
                self._exec(cursor, "SELECT * FROM nodes ORDER BY node_id")

            rows = cursor.fetchall()
            return self._rows_to_dicts(rows, cursor)
        finally:
            conn.close()

    def get_node(self, node_id):
        """Get a single node."""
        if self.state_backend == "spacetimedb" and self.stdb:
            return self.stdb.get_node(node_id)
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            self._exec(cursor, "SELECT * FROM nodes WHERE node_id = ?", (node_id,))
            row = cursor.fetchone()
            return self._row_to_dict(row, cursor)
        finally:
            conn.close()

    def scan_nodes_liveness(self, silent_min=10, offline_min=60):
        """
        Scan nodes for stale heartbeats and update status.
        Returns list of alerts created.
        """
        if self.state_backend == "spacetimedb" and self.stdb:
            now = datetime.datetime.now(datetime.timezone.utc)
            alerts_created = []
            for node in self.list_nodes():
                last_hb_raw = node.get("last_heartbeat_at")
                if not last_hb_raw:
                    continue
                try:
                    last_hb = datetime.datetime.fromisoformat(str(last_hb_raw))
                    if last_hb.tzinfo is None:
                        last_hb = last_hb.replace(tzinfo=datetime.timezone.utc)
                except Exception:
                    continue

                age_minutes = (now - last_hb).total_seconds() / 60
                current_status = node.get("status", "UNKNOWN")
                if age_minutes > offline_min:
                    new_status = "OFFLINE"
                elif age_minutes > silent_min:
                    new_status = "SILENT"
                else:
                    new_status = "ONLINE"

                if new_status != current_status:
                    self.stdb.set_node_status(node.get("node_id", ""), new_status)
            return alerts_created

        conn = self.get_connection()
        cursor = conn.cursor()
        now = datetime.datetime.now(datetime.timezone.utc)
        alerts_created = []

        try:
            self._exec(cursor, "SELECT * FROM nodes")
            nodes = self._rows_to_dicts(cursor.fetchall(), cursor)

            for node in nodes:
                if not node["last_heartbeat_at"]:
                    continue

                try:
                    last_hb = datetime.datetime.fromisoformat(node["last_heartbeat_at"])
                    if last_hb.tzinfo is None:
                        last_hb = last_hb.replace(tzinfo=datetime.timezone.utc)

                    age_minutes = (now - last_hb).total_seconds() / 60
                    current_status = node["status"]
                    new_status = current_status
                    alert_kind = None

                    # State transitions
                    if age_minutes > offline_min:
                        if current_status != "OFFLINE":
                            new_status = "OFFLINE"
                            alert_kind = "NODE_OFFLINE"
                    elif age_minutes > silent_min:
                        if current_status != "SILENT" and current_status != "OFFLINE":
                            new_status = "SILENT"
                            alert_kind = "NODE_SILENT"
                    else:
                        # Recovering?
                        if current_status != "ONLINE":
                            new_status = "ONLINE"
                            # Optional: alert_kind = "NODE_RECOVERED" (disabled for now to avoid spam, or handle carefully)

                    if new_status != current_status:
                        # Update DB
                        self._exec(
                            cursor,
                            "UPDATE nodes SET status = ? WHERE node_id = ?",
                            (new_status, node["node_id"]),
                        )
                        conn.commit()  # Commit state change immediately

                        # Create Alert if needed
                        if alert_kind:
                            alert_id = str(uuid.uuid4())
                            dedupe_key = f"{alert_kind}:{node['node_id']}:{now.strftime('%Y-%m-%d')}"
                            # Daily dedupe for persistent issues? Or maybe simpler?
                            # Requirement: "Dedup requirement: only alert on state transition."
                            # Since we only enter this block on state transition, we are good.
                            # But let's use a unique dedupe key based on the *event* to allow retries but prevent spam if multiple ticks hit race.
                            # Actually `new_status != current_status` check protects us within DB transaction context (mostly).
                            # But `scan_nodes` runs every tick. If update fails, we might retry?
                            # Let's just create alert.

                            dedupe_key = f"{alert_kind}:{node['node_id']}:{int(now.timestamp())}"  # Unique per event

                            # Wait, we need to be careful. The directive says "Dedup requirement: only alert on state transition."
                            # My logic `if new_status != current_status` handles that.
                            # The `status` field in DB acts as the state store.

                            self.create_alert(
                                cursor,
                                kind=alert_kind,
                                proposal_id="N/A",  # System alert
                                node_id=node["node_id"],
                                details={
                                    "prev_status": current_status,
                                    "age_minutes": age_minutes,
                                },
                            )
                            alerts_created.append(alert_kind)

                except Exception as e:
                    print(f"Error processing node {node['node_id']}: {e}")

            return alerts_created
        except Exception:
            conn.rollback()
            raise
        finally:
            conn.close()

    def create_alert(self, cursor, kind, proposal_id, node_id=None, details=None):
        """Helper to create an alert (reused by scan_alerts usually, but helpful here)."""
        # Note: creates alert but caller must commit.
        # This helper does NOT commit, consistent with its likely use in a larger transaction.
        import uuid

        alert_id = f"ALT-{uuid.uuid4().hex[:8]}"
        now_ts = datetime.datetime.now(datetime.timezone.utc).isoformat()
        # Dedupe key logic: per node, per kind, per minute to allow frequent but not spammy alerts
        dedupe_key = f"{kind}:{node_id}:{datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%d%H%M')}"

        # Insert
        try:
            self._exec(
                cursor,
                """
                INSERT INTO alerts (id, created_at, kind, proposal_id, node_id, status, dedupe_key, details_json, claimed_at, lease_expires_at, ttl_seconds_remaining)
                VALUES (?, ?, ?, ?, ?, 'OPEN', ?, ?, NULL, NULL, 86400)
                """,
                (
                    alert_id,
                    now_ts,
                    kind,
                    proposal_id,
                    node_id,
                    dedupe_key,
                    json.dumps(details) if details else "{}",
                ),
            )
            # Note: Ignoring dedupe constraint violation (if any) or letting it fail?
            # If dedupe key exists, it will raise IntegrityError.
            # We should probably catch it or use INSERT OR IGNORE.
            # Given state transition check, collision is unlikely unless rapid flapping.
        except Exception:
            pass  # Fail safe implies we don't crash loop for one alert failure

            pass  # Fail safe implies we don't crash loop for one alert failure

    def record_consent(self, consent_data):
        """
        Record a human consent decision and transition proposal state.
        Phase 2 Gate: QUEUED -> APPROVED (or REJECTED)
        """
        if self.state_backend == "spacetimedb" and self.stdb:
            return self.stdb.record_consent(consent_data)
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            normalized_decision = str(consent_data["decision"]).upper()
            approved_at = consent_data.get("approved_at")
            expires_at = consent_data.get("expires_at")
            if normalized_decision == "APPROVE" and not approved_at:
                approved_at = datetime.datetime.now(datetime.timezone.utc).isoformat()

            # 1. Insert Consent Record
            if self.use_postgres:
                self._exec(cursor, 
                    """
                    INSERT INTO proposal_consents 
                    (consent_id, proposal_id, proposal_hash, actor_type, actor_id, decision, comment, created_at, metadata)
                    VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s)
                    """,
                    (
                        consent_data["consent_id"],
                        consent_data["proposal_id"],
                        consent_data.get("proposal_hash", "UNKNOWN"),
                        consent_data["actor_type"],
                        consent_data["actor_id"],
                        normalized_decision,
                        consent_data.get("comment"),
                        datetime.datetime.now(datetime.timezone.utc).isoformat(),
                        json.dumps(consent_data.get("metadata", {}))
                    )
                )
            else:
                self._exec(cursor, 
                    """
                    INSERT INTO proposal_consents 
                    (consent_id, proposal_id, proposal_hash, actor_type, actor_id, decision, comment, created_at, metadata)
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
                    """,
                    (
                        consent_data["consent_id"],
                        consent_data["proposal_id"],
                        consent_data.get("proposal_hash", "UNKNOWN"),
                        consent_data["actor_type"],
                        consent_data["actor_id"],
                        normalized_decision,
                        consent_data.get("comment"),
                        datetime.datetime.now(datetime.timezone.utc).isoformat(),
                        json.dumps(consent_data.get("metadata", {}))
                    )
                )

            # 2. Transition Proposal
            new_status = "APPROVED" if normalized_decision == "APPROVE" else "REJECTED"
            
            if self.use_postgres:
                self._exec(
                    cursor,
                    "UPDATE proposals SET status = %s, approved_at = %s, expires_at = %s, proposal_hash = %s WHERE proposal_id = %s",
                    (
                        new_status,
                        approved_at,
                        expires_at,
                        consent_data.get("proposal_hash", "UNKNOWN"),
                        consent_data["proposal_id"],
                    ),
                )
            else:
                self._exec(
                    cursor,
                    "UPDATE proposals SET status = ?, approved_at = ?, expires_at = ?, proposal_hash = ? WHERE proposal_id = ?",
                    (
                        new_status,
                        approved_at,
                        expires_at,
                        consent_data.get("proposal_hash", "UNKNOWN"),
                        consent_data["proposal_id"],
                    ),
                )

            conn.commit()
            return True
        except Exception as e:
            print(f"Consent Error: {e}")
            conn.rollback()
            return False
        finally:
            conn.close()

    def get_model_usage_summary(self, minutes=60):
        """Fetch model usage stats for the last N minutes."""
        if self.state_backend == "spacetimedb" and self.stdb:
            return self.stdb.get_model_usage_summary(minutes=minutes)

        conn = self.get_connection()
        cursor = conn.cursor()
        cutoff = (
            datetime.datetime.now()
            - datetime.timedelta(minutes=minutes)
        ).isoformat()

        try:
            self._exec(
                cursor,
                """
                SELECT model_id, COUNT(*) as request_count, SUM(tokens_total) as total_tokens, SUM(cost) as total_cost
                FROM runs
                WHERE ended_at > ? AND model_id IS NOT NULL
                GROUP BY model_id
            """,
                (cutoff,),
            )
            rows = cursor.fetchall()
            return self._rows_to_dicts(rows, cursor)
        finally:
            conn.close()

    def get_last_successful_tick(self):
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            self._exec(
                cursor,
                """
                SELECT * FROM ticks 
                WHERE status = 'OK' 
                ORDER BY ended_at DESC 
                LIMIT 1
            """,
            )
            row = cursor.fetchone()
            return self._row_to_dict(row, cursor)
        finally:
            conn.close()

    def add_proposal(self, proposal):
        if self.state_backend == "spacetimedb" and self.stdb:
            return self.stdb.add_proposal(proposal)
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            cols = [
                "proposal_id",
                "created_at",
                "status",
                "fingerprint",
                "payload",
                "payload_raw",
                "mode",
            ]
            vals = [
                proposal["proposal_id"],
                proposal.get("created_at") or datetime.datetime.now(datetime.timezone.utc).isoformat(),
                proposal.get("status", "QUEUED"),
                proposal.get("fingerprint"),
                (
                    json.dumps(proposal["payload"])
                    if isinstance(proposal["payload"], (dict, list))
                    else proposal["payload"]
                ),
                proposal.get("payload_raw"),
                proposal.get("mode", "PRODUCTION"),
            ]

            optional_columns = [
                "execution_targeting",
                "assigned_node_id",
                "assignment_expires_at",
                "proposal_hash",
                "hub_signature",
                "approved_at",
                "expires_at",
                "eligibility_snapshot",
                "attempt_count",
            ]
            for column in optional_columns:
                if proposal.get(column) is None:
                    continue
                cols.append(column)
                value = proposal.get(column)
                if isinstance(value, (dict, list)):
                    value = json.dumps(value)
                vals.append(value)

            placeholders = ", ".join(["?"] * len(vals))
            col_str = ", ".join(cols)
            
            self._exec(
                cursor,
                f"INSERT INTO proposals ({col_str}) VALUES ({placeholders})",
                tuple(vals),
            )
            conn.commit()
            return True
        except Exception as e:
            # Handle both SQLite and Postgres integrity errors
            if "IntegrityError" in type(e).__name__ or "duplicate" in str(e).lower():
                return False
            raise
        finally:
            conn.close()

    def get_next_proposal(self, node_id):
        """Atomically claim the next proposal for this node."""
        if self.state_backend == "spacetimedb" and self.stdb:
            proposal = self.stdb.claim_next_approved_proposal(node_id)
            if not proposal:
                return None
            lease_expires_at = proposal.get("lease_expires_at") or (
                datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(minutes=30)
            ).isoformat()
            lease_data = self._issue_stdb_capability_lease(proposal, node_id, lease_expires_at)
            if lease_data:
                proposal["lease_id"] = lease_data["lease_id"]
                proposal["lease_token"] = lease_data["lease_id"]
            return proposal
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            if self.use_postgres:
                # Postgres: Use FOR UPDATE SKIP LOCKED for atomic claim
                self._exec(
                    cursor,
                    """
                    SELECT * FROM proposals 
                    WHERE status = 'APPROVED' 
                    ORDER BY created_at ASC 
                    LIMIT 1
                    FOR UPDATE SKIP LOCKED
                """,
                )
            else:
                # SQLite: Use exclusive transaction
                cursor.execute("BEGIN EXCLUSIVE")
                cursor.execute(
                    """
                    SELECT * FROM proposals 
                    WHERE status = 'APPROVED' 
                    ORDER BY created_at ASC 
                    LIMIT 1
                """
                )
            row = cursor.fetchone()

            if row:
                row_dict = self._row_to_dict(row, cursor)
                now = datetime.datetime.now()
                # Default lease TTL: 30 minutes
                lease_expires = (now + datetime.timedelta(minutes=30)).isoformat()
                now_iso = now.isoformat()
                self._exec(
                    cursor,
                    """
                    UPDATE proposals 
                    SET status = 'CLAIMED', node_id = ?, claimed_at = ?, lease_expires_at = ?
                    WHERE proposal_id = ?
                """,
                    (node_id, now_iso, lease_expires, row_dict["proposal_id"]),
                )
                # Return with claim metadata
                row_dict["node_id"] = node_id
                row_dict["claimed_at"] = now_iso
                row_dict["lease_expires_at"] = lease_expires
                conn.commit()
                return row_dict

            conn.commit()
            return None
        except Exception as e:
            print(f"DB Claim Error: {e}")
            conn.rollback()
            return None
        finally:
            conn.close()

    def record_run(self, run_data):
        if self.state_backend == "spacetimedb" and self.stdb:
            return self.stdb.record_run(run_data)
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            self._exec(
                cursor,
                """
                INSERT INTO runs (run_id, proposal_id, started_at, ended_at, status, chain_result, signals, artifact_index, node_id, replay_receipt, mode, model_id, tokens_input, tokens_output, tokens_total, cost)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
                (
                    run_data["run_id"],
                    run_data["proposal_id"],
                    run_data.get("started_at"),
                    datetime.datetime.now().isoformat(),
                    run_data["status"],
                    json.dumps(run_data.get("chain_result")),
                    json.dumps(run_data.get("signals")),
                    json.dumps(run_data.get("artifact_index")),
                    run_data.get("node_id"),
                    json.dumps(run_data.get("replay_receipt")),
                    run_data.get("mode"),
                    run_data.get("model_id"),
                    run_data.get("tokens_input"),
                    run_data.get("tokens_output"),
                    run_data.get("tokens_total"),
                    run_data.get("cost"),
                ),
            )

            # Update proposal status
            is_success = run_data["status"] in ["PASS", "SUCCESS", "COMPLETED"]
            final_status = "COMPLETED" if is_success else "FAILED"
            self._exec(
                cursor,
                """UPDATE proposals SET status = ? WHERE proposal_id = ?""",
                (final_status, run_data["proposal_id"]),
            )

            conn.commit()
            return True
        except Exception as e:
            print(f"DB Error: {e}")
            return False
        finally:
            conn.close()

    def requeue_proposal(self, proposal_id):
        """Operator-initiated requeue. Clears claim and resets to QUEUED."""
        if self.state_backend == "spacetimedb" and self.stdb:
            success = self.stdb.requeue_proposal(proposal_id)
            if success:
                return {"success": True}
            return {"success": False, "error": "Failed to requeue proposal"}
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            # Check current status
            cursor.execute(
                "SELECT status FROM proposals WHERE proposal_id = ?", (proposal_id,)
            )
            row = cursor.fetchone()
            if not row:
                return {"success": False, "error": "Proposal not found"}

            status = row["status"]
            if status not in ["CLAIMED", "DEAD", "FAILED"]:
                return {
                    "success": False,
                    "error": f"Cannot requeue from status {status}",
                }

            cursor.execute(
                """
                UPDATE proposals 
                SET status = 'QUEUED', node_id = NULL, claimed_at = NULL, lease_expires_at = NULL
                WHERE proposal_id = ?
            """,
                (proposal_id,),
            )
            conn.commit()
            return {"success": True}
        except Exception as e:
            print(f"Requeue Error: {e}")
            return {"success": False, "error": str(e)}
        finally:
            conn.close()

    def update_heartbeat(self, proposal_id, node_id, node_instance_id, ts_iso, detail):
        if self.state_backend == "spacetimedb" and self.stdb:
            proposal = self.stdb.record_proposal_heartbeat(
                proposal_id,
                node_id,
                node_instance_id,
                ts_iso,
                detail,
            )
            if not proposal:
                return None, "NOT_FOUND_OR_REJECTED"

            lease = self.stdb.get_active_capability_lease(proposal_id, node_id)
            if lease:
                self.stdb.renew_capability_lease(
                    lease["lease_id"],
                    ts_iso,
                    proposal.get("lease_expires_at") or lease.get("expires_at") or ts_iso,
                )
            return proposal, None
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            cursor.execute(
                "SELECT status, node_id FROM proposals WHERE proposal_id = ?",
                (proposal_id,),
            )
            row = cursor.fetchone()
            if not row:
                return None, "NOT_FOUND"
            if row["status"] != "CLAIMED":
                return None, "NOT_CLAIMED"
            if row["node_id"] != node_id:
                return None, "NODE_MISMATCH"

            cursor.execute(
                """
                UPDATE proposals
                SET last_heartbeat_at = ?, last_heartbeat_node_id = ?, last_heartbeat_node_instance_id = ?, last_heartbeat_detail = ?
                WHERE proposal_id = ?
            """,
                (
                    ts_iso,
                    node_id,
                    node_instance_id,
                    json.dumps(detail) if detail is not None else None,
                    proposal_id,
                ),
            )
            conn.commit()
            cursor.execute(
                """
                SELECT proposal_id, status, node_id, claimed_at, lease_expires_at, last_heartbeat_at
                FROM proposals WHERE proposal_id = ?
            """,
                (proposal_id,),
            )
            result = cursor.fetchone()
            return dict(result), None
        finally:
            conn.close()

    def get_proposals(self, status=None, limit=50):
        if self.state_backend == "spacetimedb" and self.stdb:
            return self.stdb.get_proposals(status=status, limit=limit)
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            if status:
                self._exec(
                    cursor,
                    "SELECT * FROM proposals WHERE status = ? ORDER BY created_at DESC LIMIT ?",
                    (status, limit),
                )
            else:
                self._exec(
                    cursor,
                    "SELECT * FROM proposals ORDER BY created_at DESC LIMIT ?",
                    (limit,),
                )
            return self._rows_to_dicts(cursor.fetchall(), cursor)
        finally:
            conn.close()

    def get_proposal(self, proposal_id):
        if self.state_backend == "spacetimedb" and self.stdb:
            return self.stdb.get_proposal(proposal_id)
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            self._exec(
                cursor,
                "SELECT * FROM proposals WHERE proposal_id = ?",
                (proposal_id,),
            )
            row = cursor.fetchone()
            return self._row_to_dict(row, cursor)
        finally:
            conn.close()

    def get_runs(self, proposal_id=None, limit=50):
        if self.state_backend == "spacetimedb" and self.stdb:
            return self.stdb.get_runs(proposal_id=proposal_id, limit=limit)

        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            if proposal_id:
                self._exec(
                    cursor,
                    "SELECT * FROM runs WHERE proposal_id = ? ORDER BY ended_at DESC LIMIT ?",
                    (proposal_id, limit),
                )
            else:
                self._exec(
                    cursor,
                    "SELECT * FROM runs ORDER BY ended_at DESC LIMIT ?",
                    (limit,),
                )
            return self._rows_to_dicts(cursor.fetchall(), cursor)
        finally:
            conn.close()

    def get_run(self, run_id):
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            self._exec(cursor, "SELECT * FROM runs WHERE run_id = ?", (run_id,))
            row = cursor.fetchone()
            return self._row_to_dict(row, cursor)
        finally:
            conn.close()

    def insert_alert(self, alert):
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            cursor.execute(
                """
                INSERT INTO alerts (
                    id, created_at, kind, proposal_id, node_id, claimed_at,
                    lease_expires_at, ttl_seconds_remaining, status, dedupe_key, details_json
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
                (
                    alert["id"],
                    alert["created_at"],
                    alert["kind"],
                    alert["proposal_id"],
                    alert.get("node_id"),
                    alert.get("claimed_at"),
                    alert.get("lease_expires_at"),
                    alert.get("ttl_seconds_remaining"),
                    alert.get("status", "OPEN"),
                    alert["dedupe_key"],
                    json.dumps(alert.get("details_json")),
                ),
            )
            conn.commit()
            return True
        except sqlite3.IntegrityError:
            return False
        finally:
            conn.close()

    def get_alerts(self, status=None, limit=50):
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            if status:
                cursor.execute(
                    """
                    SELECT * FROM alerts
                    WHERE status = ?
                    ORDER BY created_at DESC
                    LIMIT ?
                """,
                    (status, limit),
                )
            else:
                cursor.execute(
                    """
                    SELECT * FROM alerts
                    ORDER BY created_at DESC
                    LIMIT ?
                """,
                    (limit,),
                )
            rows = [dict(r) for r in cursor.fetchall()]
            for r in rows:
                if r.get("details_json"):
                    try:
                        r["details_json"] = json.loads(r["details_json"])
                    except Exception:
                        pass
            return rows
        finally:
            conn.close()

    def update_alert_status(self, alert_id, new_status):
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            cursor.execute(
                "UPDATE alerts SET status = ? WHERE id = ?", (new_status, alert_id)
            )
            conn.commit()
            return cursor.rowcount > 0
        finally:
            conn.close()

    def get_ops_snapshot(self, limit=20):
        def ensure_aware(dt):
            if dt.tzinfo is None:
                return dt.replace(tzinfo=datetime.timezone.utc)
            return dt

        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            # 1. Alerts Summary
            cursor.execute(
                """
                SELECT kind, status, COUNT(*) as count 
                FROM alerts 
                GROUP BY kind, status
            """
            )
            alerts_summary = [dict(r) for r in cursor.fetchall()]

            # 2. Claimed Proposals (TTL Board)
            cursor.execute(
                """
                SELECT proposal_id, node_id, claimed_at, lease_expires_at, last_heartbeat_at
                FROM proposals
                WHERE status = 'CLAIMED'
                ORDER BY lease_expires_at ASC
                LIMIT ?
            """,
                (limit,),
            )
            rows = cursor.fetchall()

            now = datetime.datetime.now(datetime.timezone.utc)
            claimed_proposals = []

            for row in rows:
                p_data = dict(row)

                # Calculate TTL
                lease_raw = p_data.get("lease_expires_at")
                ttl_remaining = None
                if lease_raw:
                    try:
                        lease_dt = ensure_aware(
                            datetime.datetime.fromisoformat(lease_raw)
                        )
                        ttl_remaining = int((lease_dt - now).total_seconds())
                    except:
                        pass
                p_data["ttl_seconds_remaining"] = ttl_remaining

                # Calculate Heartbeat Age
                hb_raw = p_data.get("last_heartbeat_at")
                hb_age = None
                if hb_raw:
                    try:
                        hb_dt = ensure_aware(datetime.datetime.fromisoformat(str(hb_raw)))
                        hb_age = int((now - hb_dt).total_seconds())
                    except Exception:
                        pass
                p_data["heartbeat_age_seconds"] = hb_age

                claimed_proposals.append(p_data)

            return {
                "alerts_summary": alerts_summary,
                "claimed_proposals": claimed_proposals,
                "snapshot_at": now.isoformat(),
            }
        finally:
            conn.close()

    # --- Job Queue Core (Hub-and-Spoke) ---

    def create_job(self, job_type: str, payload: Dict[str, Any]) -> Optional[str]:
        """Create a new job in PENDING state."""
        conn: sqlite3.Connection = self.get_connection()
        cursor: sqlite3.Cursor = conn.cursor()
        job_id = f"job-{uuid.uuid4()}"
        now = datetime.datetime.now(datetime.timezone.utc).isoformat()
        try:
            self._exec(
                cursor,
                """
                INSERT INTO jobs (job_id, type, status, payload_json, created_at)
                VALUES (?, ?, 'PENDING', ?, ?)
            """,
                (job_id, job_type, json.dumps(payload), now),
            )
            conn.commit()
            return job_id
        except Exception as e:
            print(f"Create Job Error: {e}")
            conn.rollback()
            return None
        finally:
            conn.close()

    def claim_job(self, node_id, job_types=None):
        """
        Atomically claim the next available job.
        Optionally filter by job_type list.
        Uses FOR UPDATE SKIP LOCKED on Postgres.
        """
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            now = datetime.datetime.now(datetime.timezone.utc)
            now_iso = now.isoformat()
            lease_expires = (now + datetime.timedelta(minutes=10)).isoformat()  # 10 min default lease

            if self.use_postgres:
                # Postgres Atomic Claim
                type_filter = ""
                params = []
                if job_types:
                    placeholders = ",".join(["%s"] * len(job_types))
                    type_filter = f"AND type IN ({placeholders})"
                    params.extend(job_types)

                query = f"""
                    SELECT * FROM jobs 
                    WHERE status = 'PENDING' 
                    {type_filter}
                    ORDER BY created_at ASC 
                    LIMIT 1
                    FOR UPDATE SKIP LOCKED
                """
                self._exec(cursor, query, tuple(params))
            else:
                # SQLite Atomic Claim (Simulated)
                cursor.execute("BEGIN EXCLUSIVE")
                type_filter = ""
                params = []
                if job_types:
                    placeholders = ",".join(["?"] * len(job_types))
                    type_filter = f"AND type IN ({placeholders})"
                    params.extend(job_types)

                query = f"""
                    SELECT * FROM jobs 
                    WHERE status = 'PENDING' 
                    {type_filter}
                    ORDER BY created_at ASC 
                    LIMIT 1
                """
                self._exec(cursor, query, tuple(params))
            
            row = cursor.fetchone()
            if row:
                job = self._row_to_dict(row, cursor)
                self._exec(
                    cursor,
                    """
                    UPDATE jobs 
                    SET status = 'PROCESSING', claimed_by_node_id = ?, claimed_at = ?, heartbeat_at = ?, lease_expires_at = ?
                    WHERE job_id = ?
                """,
                    (node_id, now_iso, now_iso, lease_expires, job["job_id"]),
                )
                job["claimed_by_node_id"] = node_id
                job["claimed_at"] = now_iso
                job["lease_expires_at"] = lease_expires
                conn.commit()
                # Parse payload for convenience
                try:
                    job["payload"] = json.loads(job["payload_json"])
                except:
                    job["payload"] = {}
                return job
            
            conn.commit()
            return None
        except Exception as e:
            print(f"Claim Job Error: {e}")
            conn.rollback()
            return None
        finally:
            conn.close()

    def heartbeat_job(self, job_id, node_id):
        """Update job heartbeat to prevent lease expiry."""
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            now = datetime.datetime.now(datetime.timezone.utc)
            lease_expires = (now + datetime.timedelta(minutes=10)).isoformat()
            
            self._exec(
                cursor,
                """
                UPDATE jobs 
                SET heartbeat_at = ?, lease_expires_at = ?
                WHERE job_id = ? AND claimed_by_node_id = ? AND status = 'PROCESSING'
            """,
                (now.isoformat(), lease_expires, job_id, node_id),
            )
            conn.commit()
            return cursor.rowcount > 0
        finally:
            conn.close()

    def finish_job(self, job_id, result, success=True, error=None):
        """Mark job as DONE or FAILED."""
        conn = self.get_connection()
        cursor = conn.cursor()
        status = "DONE" if success else "FAILED"
        try:
            self._exec(
                cursor,
                """
                UPDATE jobs 
                SET status = ?, result_json = ?, error_message = ?
                WHERE job_id = ?
            """,
                (status, json.dumps(result), error, job_id),
            )
            conn.commit()
            return True
        finally:
            conn.close()

    def requeue_dead_jobs(self, timeout_minutes=15):
        """Reset PROCESSING jobs that haven't heartbeated recently."""
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            now = datetime.datetime.now(datetime.timezone.utc)
            # Find jobs where heartbeat is too old
            now_iso = now.isoformat()
            
            self._exec(
                cursor,
                """
                UPDATE jobs
                SET status = 'PENDING', claimed_by_node_id = NULL, claimed_at = NULL, heartbeat_at = NULL, lease_expires_at = NULL, error_message = 'LEASE_EXPIRED'
                WHERE status = 'PROCESSING' AND lease_expires_at < ?
            """,
                (now_iso,),
            )
            count = cursor.rowcount
            conn.commit()
            return count
        except Exception as e:
            print(f"Requeue Jobs Error: {e}")
            return 0
        finally:
             conn.close()

    # --- Hive Mind Stream (Cognitive Blackboard) ---

    def insert_thought(self, thought):
        """Write a thought to the cognitive stream."""
        conn = self.get_connection()
        cursor = conn.cursor()
        now = datetime.datetime.now(datetime.timezone.utc).isoformat()
        try:
            if self.use_postgres:
                self._exec(cursor, """
                    INSERT INTO stream (stream_id, created_at, origin, intent, thought_type, reasoning, artifact, confidence, parent_id, tags, metadata)
                    VALUES (%s, %s, %s, %s, %s, %s, %s, %s, %s, %s, %s)
                """, (
                    thought.stream_id, now, thought.origin, thought.intent, 
                    thought.thought_type, thought.reasoning, 
                    json.dumps(thought.artifact) if thought.artifact else None,
                    thought.confidence, thought.parent_id,
                    json.dumps(thought.tags) if thought.tags else None,
                    json.dumps(thought.metadata) if thought.metadata else None
                ))
            else:
                self._exec(cursor, """
                    INSERT INTO stream (stream_id, created_at, origin, intent, thought_type, reasoning, artifact, confidence, parent_id, tags, metadata)
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """, (
                    thought.stream_id, now, thought.origin, thought.intent, 
                    thought.thought_type, thought.reasoning, 
                    json.dumps(thought.artifact) if thought.artifact else None,
                    thought.confidence, thought.parent_id,
                    json.dumps(thought.tags) if thought.tags else None,
                    json.dumps(thought.metadata) if thought.metadata else None
                ))
            conn.commit()
            return True
        except Exception as e:
            print(f"Insert Thought Error: {e}")
            conn.rollback()
            return False
        finally:
            conn.close()

    def get_stream_context(self, limit=10):
        """Fetch the latest thoughts from the stream for context."""
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            self._exec(cursor, "SELECT * FROM stream ORDER BY created_at DESC LIMIT ?", (limit,))
            rows = cursor.fetchall()
            return self._rows_to_dicts(rows, cursor)
        finally:
            conn.close()

    def get_agent_reputation(self, agent_name):
        """Calculate a trust multiplier based on historical PASS/FAIL ratio."""
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            self._exec(cursor, """
                SELECT 
                    COUNT(CASE WHEN status IN ('PASS', 'SUCCESS', 'COMPLETED') THEN 1 END) as success_count,
                    COUNT(*) as total_count
                FROM runs
                WHERE node_id = ? OR model_id = ?
            """, (agent_name, agent_name))
            row = cursor.fetchone()
            if not row or row[1] == 0:
                return 1.0 # Default neutral trust
            
            success_rate = row[0] / row[1]
            # Multiplier ranges from 0.8 (poor) to 1.2 (excellent)
            return 0.8 + (success_rate * 0.4)
        finally:
            conn.close()

    def get_discord_channel(self, purpose: str) -> Optional[int]:
        """Fetch channel ID by purpose with SpacetimeDB fallback."""
        if self.state_backend == "spacetimedb" and self.stdb:
            channel_id = self.stdb.get_discord_channel(purpose)
            if channel_id is not None:
                return channel_id

        # 1. Local Cache Check
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            self._exec(cursor, "SELECT channel_id FROM discord_channels WHERE purpose = ?", (purpose,))
            row = cursor.fetchone()
            if row:
                return int(row[0])
        except Exception as e:
            print(f"[DB] Local channel lookup failed: {e}")
        finally:
            conn.close()

        # 2. SpacetimeDB Source of Truth Check
        try:
            res = self.stdb.query(f"SELECT channel_id FROM discord_channels WHERE purpose = '{purpose}'")
            if res and "channel_id" in res[0]:
                return int(res[0]["channel_id"])
        except Exception as e:
            print(f"[DB] STDB channel lookup failed: {e}")

        # 3. Environment Fallback
        env_map = {
            "operator-input": os.getenv("HEIWA_OPERATOR_INPUT_CHANNEL_ID"),
            "operator-ingress": os.getenv("DISCORD_CHANNEL_ID"),
            "central-comms": os.getenv("DISCORD_CHANNEL_ID"),
        }
        val = env_map.get(purpose)
        if val:
            try: return int(val)
            except: pass

    def record_route_decision(self, route_data: Dict[str, Any]) -> bool:
        if self.state_backend == "spacetimedb" and self.stdb:
            return self.stdb.record_route_decision(route_data)
        return True

        return None

    def upsert_discord_channel(self, purpose, channel_id, category_name=None, permissions=None):
        """Register or update a channel mapping."""
        conn = self.get_connection()
        cursor = conn.cursor()
        now = datetime.datetime.now(datetime.timezone.utc).isoformat()
        try:
            if self.use_postgres:
                self._exec(cursor, """
                    INSERT INTO discord_channels (purpose, channel_id, category_name, permissions_json, last_verified_at)
                    VALUES (%s, %s, %s, %s, %s)
                    ON CONFLICT (purpose) DO UPDATE SET channel_id = %s, category_name = %s, permissions_json = %s, last_verified_at = %s
                """, (purpose, channel_id, category_name, json.dumps(permissions), now, channel_id, category_name, json.dumps(permissions), now))
            else:
                self._exec(cursor, """
                    INSERT INTO discord_channels (purpose, channel_id, category_name, permissions_json, last_verified_at)
                    VALUES (?, ?, ?, ?, ?)
                    ON CONFLICT (purpose) DO UPDATE SET channel_id = ?, category_name = ?, permissions_json = ?, last_verified_at = ?
                """, (purpose, channel_id, category_name, json.dumps(permissions), now, channel_id, category_name, json.dumps(permissions), now))
            conn.commit()
            return True
        except Exception as e:
            print(f"Discord Channel Error: {e}")
            conn.rollback()
            return False
        finally:
            conn.close()

    def get_discord_role(self, role_name):
        """Fetch role ID by name."""
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            self._exec(cursor, "SELECT role_id FROM discord_roles WHERE role_name = ?", (role_name,))
            row = cursor.fetchone()
            return row[0] if row else None
        finally:
            conn.close()

    def upsert_discord_role(self, role_name, role_id, permissions_bitmask=None):
        """Register or update a role mapping."""
        conn = self.get_connection()
        cursor = conn.cursor()
        now = datetime.datetime.now(datetime.timezone.utc).isoformat()
        try:
            if self.use_postgres:
                self._exec(cursor, """
                    INSERT INTO discord_roles (role_name, role_id, permissions_bitmask, last_verified_at)
                    VALUES (%s, %s, %s, %s)
                    ON CONFLICT (role_name) DO UPDATE SET role_id = %s, permissions_bitmask = %s, last_verified_at = %s
                """, (role_name, role_id, permissions_bitmask, now, role_id, permissions_bitmask, now))
            else:
                self._exec(cursor, """
                    INSERT INTO discord_roles (role_name, role_id, permissions_bitmask, last_verified_at)
                    VALUES (?, ?, ?, ?)
                    ON CONFLICT (role_name) DO UPDATE SET role_id = ?, permissions_bitmask = ?, last_verified_at = ?
                """, (role_name, role_id, permissions_bitmask, now, role_id, permissions_bitmask, now))
            conn.commit()
            return True
        except Exception as e:
            print(f"Discord Role Error: {e}")
            conn.rollback()
            return False
        finally:
            conn.close()

    def scan_alerts(self, now_utc):
        """
        Scan for all L4 detections:
        1. LEASE_EXPIRED / EXPIRING_SOON
        2. HEARTBEAT_STALE (> 5m)
        3. PROPOSAL_STUCK_CLAIMED (> 2*Lease)
        4. RUN_FAILURE_SPIKE (> 3 failures in 1h)
        5. SIGNAL_TRUNCATED_SEEN (in last 1h)
        """

        def ensure_aware(dt):
            if dt.tzinfo is None:
                return dt.replace(tzinfo=datetime.timezone.utc)
            return dt

        now = ensure_aware(now_utc)
        conn = self.get_connection()
        cursor = conn.cursor()
        created = 0
        try:
            # --- 1, 2, 3: Claimed Proposal Scans ---
            self._exec(
                cursor,
                """
                SELECT proposal_id, node_id, claimed_at, lease_expires_at, last_heartbeat_at
                FROM proposals
                WHERE status = 'CLAIMED'
            """,
            )
            rows = cursor.fetchall()
            rows = self._rows_to_dicts(rows, cursor) if rows else []
            for row in rows:
                p_id = row["proposal_id"]
                n_id = row["node_id"]

                # Parse Timestamps
                lease_raw = row["lease_expires_at"]
                hb_raw = row["last_heartbeat_at"]

                # Default lease duration estimation (if not stored) -> 30m?
                # Better: calculate implied duration or just assume reasonable default for stuck checks.
                # Actually, standard lease is 30m.
                LEASE_DURATION_SEC = 1800

                lease_dt = None
                hb_dt = None
                claimed_dt = None

                try:
                    if lease_raw:
                        lease_dt = ensure_aware(
                            datetime.datetime.fromisoformat(lease_raw)
                        )
                    if hb_raw:
                        hb_dt = ensure_aware(datetime.datetime.fromisoformat(hb_raw))
                    if row["claimed_at"]:
                        claimed_dt = ensure_aware(
                            datetime.datetime.fromisoformat(row["claimed_at"])
                        )
                except:
                    continue  # Skip malformed rows

                alerts = []

                # Rule 1: Lease Expiry
                if lease_dt:
                    ttl = int((lease_dt - now).total_seconds())
                    if ttl <= 0:
                        alerts.append(("LEASE_EXPIRED", f"ttl={ttl}"))

                # Rule 2: Heartbeat Stale
                if hb_dt:
                    hb_age = (now - hb_dt).total_seconds()
                    if hb_age > 300:  # 5 minutes
                        alerts.append(("HEARTBEAT_STALE", f"age={int(hb_age)}s"))
                elif claimed_dt:
                    # No heartbeat ever, compare to claimed
                    # If claimed > 5 mins ago and no heartbeat -> STALE
                    if (now - claimed_dt).total_seconds() > 300:
                        alerts.append(("HEARTBEAT_STALE", "never_seen"))

                # Rule 3: Proposal Stuck Claimed
                if claimed_dt:
                    stuck_age = (now - claimed_dt).total_seconds()
                    if stuck_age > (2 * LEASE_DURATION_SEC):  # 1 hour
                        alerts.append(
                            ("PROPOSAL_STUCK_CLAIMED", f"age={int(stuck_age)}s")
                        )

                # Ingest alerts for this proposal
                for kind, details in alerts:
                    # Dedupe key: Proposal + Kind + Hour (or specific event window)
                    # For state-based alerts, we want to alert once per "state instance".
                    # Using claimed_at or lease_expires_at as unique anchor?
                    # Or just naive hourly dedupe.
                    # The previous key was f"{row['proposal_id']}:{kind}:{lease_raw}"
                    # Let's stick to that for lease.
                    # For HB/Stuck, we can use 30-min window buckets.
                    window = now.strftime("%Y%m%d%H") + (
                        "00" if now.minute < 30 else "30"
                    )
                    dedupe_key = f"{p_id}:{kind}:{window}"

                    self._insert_alert(
                        cursor, kind, p_id, n_id, dedupe_key, {"details": details}
                    )
                    created += 1

            # --- 4. Run Failure Spike ---
            self._exec(
                cursor,
                """
                SELECT count(*) as fail_count 
                FROM runs 
                WHERE status = 'FAILED' 
                AND ended_at > ?
                """,
                ((now - datetime.timedelta(hours=1)).isoformat(),),
            )
            fail_row = cursor.fetchone()
            fail_count = (
                self._row_to_dict(fail_row, cursor)["fail_count"] if fail_row else 0
            )
            if fail_count >= 3:
                # Dedupe by hour
                dedupe_key = f"SYSTEM:RUN_FAILURE_SPIKE:{now.strftime('%Y%m%d%H')}"
                self._insert_alert(
                    cursor,
                    "RUN_FAILURE_SPIKE",
                    "SYSTEM",
                    None,
                    dedupe_key,
                    {"count": fail_count},
                )
                created += 1

            # --- 5. Signal Truncated Seen ---
            self._exec(
                cursor,
                """
                SELECT run_id 
                FROM runs 
                WHERE ended_at > ? 
                AND signals LIKE ?
                LIMIT 1
                """,
                (
                    (now - datetime.timedelta(hours=1)).isoformat(),
                    '%"kind": "TRUNCATED"%',
                ),
            )
            trunc_row = cursor.fetchone()
            trunc_run = self._row_to_dict(trunc_row, cursor) if trunc_row else None
            if trunc_run:
                dedupe_key = f"SYSTEM:SIGNAL_TRUNCATED_SEEN:{now.strftime('%Y%m%d%H')}"
                self._insert_alert(
                    cursor,
                    "SIGNAL_TRUNCATED_SEEN",
                    "SYSTEM",
                    None,
                    dedupe_key,
                    {"example_run": trunc_run["run_id"]},
                )
                created += 1

            conn.commit()
            return created
        except Exception:
            conn.rollback()
            raise
        finally:
            conn.close()

    def generate_proposals_from_alerts(self):
        """
        Generate proposals from OPEN alerts.
        Maps Alert Kind -> Action Class.
        Idempotent via proposal fingerprint.
        """
        conn = self.get_connection()
        cursor = conn.cursor()
        created_count = 0
        try:
            # Get OPEN alerts that don't have a linked proposal yet (conceptually).
            # We rely on fingerprint check for idempotency so we can just scan all OPEN alerts
            # created in the last scan window (e.g. 1 hour) or just all OPEN ones.
            # To be safe, let's scan all OPEN alerts, but only one proposal per alert ID.

            self._exec(
                cursor,
                "SELECT id, kind, proposal_id, details_json FROM alerts WHERE status = 'OPEN'",
            )
            alerts = cursor.fetchall()
            alerts = self._rows_to_dicts(alerts, cursor) if alerts else []

            mapping = {
                "LEASE_EXPIRED": "REMEDIATE",
                "HEARTBEAT_STALE": "NOTIFY",
                "PROPOSAL_STUCK_CLAIMED": "REMEDIATE",
                "RUN_FAILURE_SPIKE": "ESCALATE",
                "SIGNAL_TRUNCATED_SEEN": "NOTIFY",
            }

            for alert in alerts:
                kind = alert["kind"]
                action_class = mapping.get(kind)
                if not action_class:
                    continue

                alert_id = alert["id"]
                # Deterministic fingerprint: l4:{alert_id}
                fingerprint = f"l4:{alert_id}"

                # Check if proposal exists
                self._exec(
                    cursor,
                    "SELECT proposal_id FROM proposals WHERE fingerprint = ?",
                    (fingerprint,),
                )
                if cursor.fetchone():
                    continue

                # Economic Estimation
                economic = self.estimate_economic(kind)

                # Apply Gates
                final_action, gate_decision = self.apply_economic_gates(
                    cursor, economic, action_class
                )

                # Update Economic Block
                economic["gate"] = {
                    "decision": gate_decision,
                    "reason": f"Gate applied: {gate_decision}",
                }

                # Create Proposal
                # proposal_id: auto-<short_alert_id>
                prop_id = f"auto-{alert_id[:8]}"

                details = {}
                try:
                    details = json.loads(alert["details_json"])
                except:
                    pass

                payload = {
                    "alert_id": alert_id,
                    "alert_kind": kind,
                    "target_proposal_id": alert["proposal_id"],
                    "action_class": final_action,
                    "reason": f"L4 Auto-Generation for {kind}",
                    "evidence": details,
                    "economic": economic,
                }

                ts = datetime.datetime.now(datetime.timezone.utc).isoformat()

                # Insert Proposal
                try:
                    self._exec(
                        cursor,
                        """
                        INSERT INTO proposals (
                            proposal_id, created_at, status, payload, fingerprint, mode
                        ) VALUES (?, ?, 'OPEN', ?, ?, 'PRODUCTION')
                        """,
                        (prop_id, ts, json.dumps(payload), fingerprint),
                    )
                    created_count += 1
                except Exception as e:
                    # Handle both SQLite and Postgres integrity errors
                    if (
                        "IntegrityError" in type(e).__name__
                        or "duplicate" in str(e).lower()
                    ):
                        pass  # Collision on ID or Fingerprint
                    else:
                        raise

            conn.commit()
            return created_count
        except Exception:
            conn.rollback()
            raise
        finally:
            conn.close()

    def estimate_economic(self, kind):
        """Uncertainty Principle: Estimate cost/risk statically."""
        defaults = {
            "LEASE_EXPIRED": {
                "risk_level": "LOW",
                "estimated_minutes": 5,
                "blast_radius": "NODE",
            },
            "PROPOSAL_STUCK_CLAIMED": {
                "risk_level": "MEDIUM",
                "estimated_minutes": 10,
                "blast_radius": "NODE",
            },
            "HEARTBEAT_STALE": {
                "risk_level": "LOW",
                "estimated_minutes": 0,
                "blast_radius": "NODE",
            },
            "RUN_FAILURE_SPIKE": {
                "risk_level": "HIGH",
                "estimated_minutes": 0,
                "blast_radius": "SYSTEM",
            },
            "SIGNAL_TRUNCATED_SEEN": {
                "risk_level": "LOW",
                "estimated_minutes": 0,
                "blast_radius": "SYSTEM",
            },
        }
        return defaults.get(
            kind,
            {
                "risk_level": "HIGH",
                "estimated_minutes": 0,
                "blast_radius": "SYSTEM",
                "note": "Unknown kind",
            },
        )

    def apply_economic_gates(self, cursor, economic, desired_action):
        """
        Check budgets and downgrade if necessary.
        Caps: 5/hr Remediate, 20/day Remediate, 2/day High Risk.
        Returns: (final_action, gate_decision)
        """
        # Gather Budgets
        now = datetime.datetime.utcnow()
        t_1h = (now - datetime.timedelta(hours=1)).isoformat()
        t_24h = (now - datetime.timedelta(hours=24)).isoformat()

        # Count REMEDIATE last hour
        self._exec(
            cursor,
            """SELECT count(*) as c FROM proposals 
               WHERE created_at > ? AND payload LIKE '%"action_class": "REMEDIATE"%'""",
            (t_1h,),
        )
        remediate_last_hour = cursor.fetchone()["c"]

        # Count REMEDIATE last day
        self._exec(
            cursor,
            """SELECT count(*) as c FROM proposals 
               WHERE created_at > ? AND payload LIKE '%"action_class": "REMEDIATE"%'""",
            (t_24h,),
        )
        remediate_last_day = cursor.fetchone()["c"]

        # Count HIGH RISK last day (action irrelevant? No, only count high risk ACTIONS? or all high risk proposals?)
        # Let's count all HIGH risk proposals generated, regardless of action?
        # Actually, user said "Max HIGH-risk remediations per day"? Or just "Max HIGH risk/day cap"?
        # Requirement: "HIGH risk/day cap: 2". Downgrade rule: "Over HIGH risk/day -> ESCALATE".
        # This implies we count HIGH risk proposals.
        self._exec(
            cursor,
            """SELECT count(*) as c FROM proposals 
               WHERE created_at > ? AND payload LIKE '%"risk_level": "HIGH"%'""",
            (t_24h,),
        )
        high_risk_last_day = cursor.fetchone()["c"]

        # Build Stats Block
        stats = {
            "remediate_last_hour": remediate_last_hour,
            "remediate_last_day": remediate_last_day,
            "high_risk_last_day": high_risk_last_day,
            "remediate_per_hour_cap": 5,
            "remediate_per_day_cap": 20,
            "high_risk_per_day_cap": 2,
        }
        economic["budget"] = stats

        # Gate Logic

        # 1. High Risk Check
        if economic["risk_level"] == "HIGH":
            if high_risk_last_day >= 2:
                return "ESCALATE", "DOWNGRADE_ESCALATE_RISK_CAP"
            # High risk defaults to ESCALATE usually, but if desired was REMEDIATE?
            # RUN_FAILURE_SPIKE is HIGH/ESCALATE by default.
            # If we had a HIGH/REMEDIATE kind (none currently), this gate would matter.
            # But the policy says: "Over HIGH risk/day -> ESCALATE".
            # So even if desired is NOTIFY, should we escalate? Probably not.
            # Only downgrade stronger actions.
            pass

        # 2. Remediation Budgets (Only applies if attempting REMEDIATE)
        if desired_action == "REMEDIATE":
            if remediate_last_hour >= 5:
                # Downgrade to NOTIFY
                # Wait, rule says: "Over REMEDIATE/hour or REMEDIATE/day -> NOTIFY"
                return "NOTIFY", "DOWNGRADE_NOTIFY_HOURLY_CAP"
            if remediate_last_day >= 20:
                return "NOTIFY", "DOWNGRADE_NOTIFY_DAILY_CAP"

        return desired_action, "ALLOW"

    def _insert_alert(self, cursor, kind, p_id, n_id, dedupe_key, details):
        a_id = str(uuid.uuid4())
        ts = datetime.datetime.now(datetime.timezone.utc).isoformat()
        try:
            self._exec(
                cursor,
                """
                INSERT INTO alerts (
                    id, created_at, kind, proposal_id, node_id, 
                    status, dedupe_key, details_json
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (a_id, ts, kind, p_id, n_id, "OPEN", dedupe_key, json.dumps(details)),
            )
        except Exception as e:
            # Handle both SQLite and Postgres integrity errors
            if "IntegrityError" in type(e).__name__ or "duplicate" in str(e).lower():
                pass  # Already exists
            else:
                raise

    def get_run_signals(self, run_id):
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            self._exec(cursor, "SELECT signals FROM runs WHERE run_id = ?", (run_id,))
            row = cursor.fetchone()
            if not row:
                return None

            row_dict = self._row_to_dict(row, cursor)
            signals = row_dict["signals"] if row_dict else None
            if not signals:
                return []

            if isinstance(signals, str):
                try:
                    return json.loads(signals)
                except:
                    return []
            return signals
        finally:
            conn.close()

    # ========== PHASE 2: CONSENT & ROUTING ==========

    def append_consent(
        self, proposal_id, proposal_hash, actor_type, actor_id, decision, comment=None
    ):
        """Append a consent record (audit log). Returns consent_id or None on failure."""
        if self.state_backend == "spacetimedb" and self.stdb:
            consent_id = f"CON-{uuid.uuid4().hex[:8]}"
            proposal = self.get_proposal(proposal_id) or {}
            now_ts = datetime.datetime.now(datetime.timezone.utc).isoformat()
            targeting = self._targeting_from_proposal(proposal)
            ttl_seconds = int(targeting.get("ttl_seconds", 3600))
            expires_at = (
                datetime.datetime.now(datetime.timezone.utc)
                + datetime.timedelta(seconds=ttl_seconds)
            ).isoformat()
            success = self.stdb.record_consent(
                {
                    "consent_id": consent_id,
                    "proposal_id": proposal_id,
                    "proposal_hash": proposal_hash,
                    "actor_type": actor_type,
                    "actor_id": actor_id,
                    "decision": decision,
                    "comment": comment,
                    "metadata": {},
                    "requested_at": proposal.get("created_at") or now_ts,
                    "request_expires_at": expires_at if str(decision).lower() == "approve" else None,
                    "request_payload": self._parse_json_field(proposal.get("payload"), proposal.get("payload")),
                    "approved_at": now_ts if str(decision).lower() == "approve" else None,
                    "expires_at": expires_at if str(decision).lower() == "approve" else None,
                }
            )
            return consent_id if success else None
        conn = self.get_connection()
        cursor = conn.cursor()
        consent_id = f"CON-{uuid.uuid4().hex[:8]}"
        now_ts = datetime.datetime.now(datetime.timezone.utc).isoformat()
        try:
            self._exec(
                cursor,
                """
                INSERT INTO proposal_consents (consent_id, proposal_id, proposal_hash, actor_type, actor_id, decision, comment, created_at)
                VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    consent_id,
                    proposal_id,
                    proposal_hash,
                    actor_type,
                    actor_id,
                    decision,
                    comment,
                    now_ts,
                ),
            )
            conn.commit()
            return consent_id
        except Exception as e:
            conn.rollback()
            print(f"[DB] Consent append error: {e}")
            return None
        finally:
            conn.close()

    def get_consents_for_proposal(self, proposal_id):
        """Get all consents for a proposal (ordered by created_at)."""
        if self.state_backend == "spacetimedb" and self.stdb:
            return self.stdb.get_consents_for_proposal(proposal_id)
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            self._exec(
                cursor,
                "SELECT * FROM proposal_consents WHERE proposal_id = ? ORDER BY created_at",
                (proposal_id,),
            )
            return self._rows_to_dicts(cursor.fetchall(), cursor)
        finally:
            conn.close()

    def transition_proposal_status(self, proposal_id, new_status, extra_fields=None):
        """
        Transition a proposal to a new status.
        extra_fields: dict of additional columns to set (e.g., approved_at, expires_at).
        """
        if self.state_backend == "spacetimedb" and self.stdb:
            extra_fields = extra_fields or {}
            proposal = self.get_proposal(proposal_id) or {}
            payload_str = proposal.get("payload") or ""
            if not isinstance(payload_str, str):
                payload_str = json.dumps(payload_str, sort_keys=True)
            proposal_hash = extra_fields.get("proposal_hash") or proposal.get("proposal_hash")
            if not proposal_hash and payload_str:
                proposal_hash = hashlib.sha256(payload_str.encode("utf-8")).hexdigest()

            if new_status == "APPROVED":
                return self.stdb.approve_proposal(
                    proposal_id,
                    proposal_hash or f"HASH-{proposal_id}",
                    extra_fields.get("approved_at")
                    or datetime.datetime.now(datetime.timezone.utc).isoformat(),
                    extra_fields.get("expires_at"),
                )
            if new_status == "REJECTED":
                return self.stdb.reject_proposal(proposal_id)
            if new_status == "ASSIGNED":
                assigned_node_id = extra_fields.get("assigned_node_id")
                assignment_expires_at = extra_fields.get("assignment_expires_at")
                if not assigned_node_id or not assignment_expires_at:
                    return False
                hub_signature = extra_fields.get("hub_signature") or proposal.get("hub_signature")
                if not hub_signature:
                    hub_signature = f"SIG-{(proposal_hash or proposal_id)[:16]}"
                return self.stdb.assign_proposal(
                    proposal_id,
                    assigned_node_id,
                    assignment_expires_at,
                    hub_signature,
                    proposal_hash or f"HASH-{proposal_id}",
                    int(extra_fields.get("attempt_count") or proposal.get("attempt_count") or 0),
                    extra_fields.get("eligibility_snapshot"),
                )
            if new_status == "QUEUED":
                return self.stdb.queue_proposal(
                    proposal_id,
                    extra_fields.get("eligibility_snapshot"),
                )
            if new_status == "EXPIRED":
                return self.stdb.expire_proposal(
                    proposal_id,
                    extra_fields.get("eligibility_snapshot"),
                )
            if new_status == "CLAIMED":
                claimed = self.stdb.claim_proposal(
                    proposal_id,
                    extra_fields.get("node_id") or proposal.get("assigned_node_id") or "",
                    extra_fields.get("claimed_at"),
                    extra_fields.get("lease_expires_at"),
                )
                return claimed is not None
            return False
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            set_clause = "status = ?"
            params = [new_status]
            if extra_fields:
                for k, v in extra_fields.items():
                    set_clause += f", {k} = ?"
                    params.append(v)
            params.append(proposal_id)
            self._exec(
                cursor,
                f"UPDATE proposals SET {set_clause} WHERE proposal_id = ?",
                tuple(params),
            )
            conn.commit()
            return cursor.rowcount > 0
        except Exception as e:
            conn.rollback()
            print(f"[DB] Proposal transition error: {e}")
            return False
        finally:
            conn.close()

    def get_eligible_nodes(self, requires, privilege_tier="cloud_safe"):
        """
        Get nodes that are ONLINE and match the required capabilities.
        Returns list of node dicts.
        """
        if self.state_backend == "spacetimedb" and self.stdb:
            return self._filter_eligible_nodes(
                self.stdb.list_nodes(status="ONLINE"),
                requires,
                privilege_tier,
            )
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            # Get ONLINE nodes
            self._exec(cursor, "SELECT * FROM nodes WHERE status = 'ONLINE'")
            nodes = self._rows_to_dicts(cursor.fetchall(), cursor)
            return self._filter_eligible_nodes(nodes, requires, privilege_tier)
        finally:
            conn.close()

    def assign_proposal_to_node(
        self,
        proposal_id,
        node_id,
        assignment_expires_at,
        proposal_hash=None,
        hub_signature=None,
        attempt_count=None,
        eligibility_snapshot=None,
    ):
        """Assign a proposal to a specific node."""
        if self.state_backend == "spacetimedb" and self.stdb:
            proposal = self.get_proposal(proposal_id) or {}
            payload_str = proposal.get("payload") or ""
            if not isinstance(payload_str, str):
                payload_str = json.dumps(payload_str, sort_keys=True)
            proposal_hash = proposal_hash or proposal.get("proposal_hash")
            if not proposal_hash and payload_str:
                proposal_hash = hashlib.sha256(payload_str.encode("utf-8")).hexdigest()
            hub_signature = hub_signature or proposal.get("hub_signature") or f"SIG-{(proposal_hash or proposal_id)[:16]}"
            return self.stdb.assign_proposal(
                proposal_id,
                node_id,
                assignment_expires_at,
                hub_signature,
                proposal_hash or f"HASH-{proposal_id}",
                int(attempt_count or proposal.get("attempt_count") or 0),
                eligibility_snapshot,
            )
        return self.transition_proposal_status(
            proposal_id,
            "ASSIGNED",
            {
                "assigned_node_id": node_id,
                "assignment_expires_at": assignment_expires_at,
                "proposal_hash": proposal_hash,
                "hub_signature": hub_signature,
                "attempt_count": attempt_count,
                "eligibility_snapshot": eligibility_snapshot,
            },
        )

    def claim_for_node(self, node_id, max_items=1):
        """
        Atomic claim (Phase 2): Only return proposals where:
        - status='ASSIGNED'
        - assigned_node_id=node_id
        - assignment_expires_at > now
        - hub_signature is not null
        Transition to CLAIMED with lease token.
        Returns list of claimed proposal dicts.
        """
        if self.state_backend == "spacetimedb" and self.stdb:
            claimed = []
            now = datetime.datetime.now(datetime.timezone.utc)
            now_iso = now.isoformat()
            rows = self.stdb.query(
                "SELECT * FROM proposals "
                "WHERE status = 'ASSIGNED' "
                f"AND assigned_node_id = '{self.stdb._escape_sql_literal(node_id)}' "
                f"AND assignment_expires_at > '{self.stdb._escape_sql_literal(now_iso)}' "
                "AND hub_signature <> '' "
                "ORDER BY created_at ASC "
                f"LIMIT {int(max_items)}"
            )
            for row in rows:
                lease_expires = (now + datetime.timedelta(minutes=30)).isoformat()
                updated = self.stdb.claim_proposal(
                    row["proposal_id"],
                    node_id,
                    now_iso,
                    lease_expires,
                )
                if not updated:
                    continue
                lease_data = self._issue_stdb_capability_lease(
                    updated,
                    node_id,
                    updated.get("lease_expires_at") or lease_expires,
                )
                if lease_data:
                    updated["lease_id"] = lease_data["lease_id"]
                    updated["lease_token"] = lease_data["lease_id"]
                claimed.append(updated)
            return claimed
        conn = self.get_connection()
        cursor = conn.cursor()
        claimed = []
        try:
            now = datetime.datetime.now(datetime.timezone.utc)
            now_iso = now.isoformat()

            if self.use_postgres:
                # Postgres: Use FOR UPDATE SKIP LOCKED for atomic claim
                self._exec(
                    cursor,
                    """
                    SELECT * FROM proposals 
                    WHERE status = 'ASSIGNED' 
                    AND assigned_node_id = ?
                    AND assignment_expires_at > ?
                    AND hub_signature IS NOT NULL
                    ORDER BY created_at ASC 
                    LIMIT ?
                    FOR UPDATE SKIP LOCKED
                """,
                    (node_id, now_iso, max_items),
                )
            else:
                # SQLite: Use exclusive transaction
                cursor.execute("BEGIN EXCLUSIVE")
                self._exec(
                    cursor,
                    """
                    SELECT * FROM proposals 
                    WHERE status = 'ASSIGNED' 
                    AND assigned_node_id = ?
                    AND assignment_expires_at > ?
                    AND hub_signature IS NOT NULL
                    ORDER BY created_at ASC 
                    LIMIT ?
                """,
                    (node_id, now_iso, max_items),
                )

            rows = cursor.fetchall()
            rows = self._rows_to_dicts(rows, cursor) if rows else []

            for row in rows:
                proposal_id = row["proposal_id"]
                # Generate lease token
                lease_token = f"LEASE-{uuid.uuid4().hex[:12]}"
                # Default lease TTL: 30 minutes
                lease_expires = (now + datetime.timedelta(minutes=30)).isoformat()

                self._exec(
                    cursor,
                    """
                    UPDATE proposals 
                    SET status = 'CLAIMED', 
                        node_id = ?, 
                        claimed_at = ?, 
                        lease_expires_at = ?
                    WHERE proposal_id = ? AND status = 'ASSIGNED'
                """,
                    (node_id, now_iso, lease_expires, proposal_id),
                )

                if cursor.rowcount > 0:
                    row["status"] = "CLAIMED"
                    row["node_id"] = node_id
                    row["claimed_at"] = now_iso
                    row["lease_expires_at"] = lease_expires
                    row["lease_token"] = lease_token
                    claimed.append(row)

            conn.commit()
            return claimed
        except Exception as e:
            print(f"[DB] Claim for node error: {e}")
            conn.rollback()
            return []
        finally:
            conn.close()

    def get_routable_proposals(self):
        """
        Get proposals that are APPROVED or QUEUED and not expired.
        Used by router tick.
        """
        if self.state_backend == "spacetimedb" and self.stdb:
            return self.stdb.get_routable_proposals()
        conn = self.get_connection()
        cursor = conn.cursor()
        try:
            now_iso = datetime.datetime.now(datetime.timezone.utc).isoformat()
            self._exec(
                cursor,
                """
                SELECT * FROM proposals 
                WHERE status IN ('APPROVED', 'QUEUED')
                AND (expires_at IS NULL OR expires_at > ?)
                ORDER BY created_at ASC
            """,
                (now_iso,),
            )
            return self._rows_to_dicts(cursor.fetchall(), cursor)
        finally:
            conn.close()

    def expire_proposal(self, proposal_id, reason=None):
        """Transition a proposal to EXPIRED status."""
        if self.state_backend == "spacetimedb" and self.stdb:
            return self.stdb.expire_proposal(
                proposal_id,
                {"expired_reason": reason or "no_eligible_nodes"},
            )
        return self.transition_proposal_status(
            proposal_id,
            "EXPIRED",
            {
                "eligibility_snapshot": json.dumps(
                    {"expired_reason": reason or "no_eligible_nodes"}
                )
            },
        )


db = Database()


# =============================================================================
# HIVE MIND STREAM API (Blackboard Access Layer)
# =============================================================================


def generate_stream_id():
    """Generate unique stream ID: THT_YYYYMMDD_HHMMSS_<RANDOM>"""
    now = datetime.datetime.now(datetime.timezone.utc)
    random_suffix = uuid.uuid4().hex[:6].upper()
    return f"THT_{now.strftime('%Y%m%d_%H%M%S')}_{random_suffix}"


class Thought:
    """Dataclass representing an agent thought on the Blackboard."""

    def __init__(
        self,
        origin: str,
        intent: str,
        thought_type: str,
        confidence: float,
        reasoning: Optional[str] = None,
        artifact: Optional[Dict[Any, Any]] = None,
        parent_id: Optional[str] = None,
        tags: Optional[List[Any]] = None,
        metadata: Optional[Dict[Any, Any]] = None,
        stream_id: Optional[str] = None,
        created_at: Optional[str] = None,
    ):
        self.stream_id = stream_id or generate_stream_id()
        self.created_at = created_at or datetime.datetime.now(
            datetime.timezone.utc
        ).isoformat()
        self.origin = origin
        self.intent = intent
        self.thought_type = thought_type
        self.reasoning = reasoning
        self.artifact = artifact or {"type": "none"}
        self.confidence = confidence
        self.parent_id = parent_id
        self.tags = tags or []
        self.metadata = metadata or {}

    def to_dict(self) -> dict:
        return {
            "stream_id": self.stream_id,
            "created_at": self.created_at,
            "origin": self.origin,
            "intent": self.intent,
            "thought_type": self.thought_type,
            "reasoning": self.reasoning,
            "artifact": self.artifact,
            "confidence": self.confidence,
            "parent_id": self.parent_id,
            "tags": self.tags,
            "metadata": self.metadata,
        }

    @classmethod
    def from_dict(cls, data: dict) -> "Thought":
        return cls(
            stream_id=data.get("stream_id"),
            created_at=data.get("created_at"),
            origin=data["origin"],
            intent=data["intent"],
            thought_type=data["thought_type"],
            reasoning=data.get("reasoning"),
            artifact=data.get("artifact"),
            confidence=data["confidence"],
            parent_id=data.get("parent_id"),
            tags=data.get("tags"),
            metadata=data.get("metadata"),
        )


# Extend Database class with stream methods
def _insert_thought(self, thought: Thought) -> bool:
    """
    Insert a Thought into the Blackboard stream.
    Returns True on success, False on failure.
    """
    conn = self.get_connection()
    cursor = conn.cursor()
    try:
        data = thought.to_dict()
        self._exec(
            cursor,
            """
            INSERT INTO stream (
                stream_id, created_at, origin, intent, thought_type,
                reasoning, artifact, confidence, parent_id, tags, metadata
            ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
            (
                data["stream_id"],
                data["created_at"],
                data["origin"],
                data["intent"],
                data["thought_type"],
                data["reasoning"],
                json.dumps(data["artifact"]) if data["artifact"] else None,
                data["confidence"],
                data["parent_id"],
                json.dumps(data["tags"]) if data["tags"] else None,
                json.dumps(data["metadata"]) if data["metadata"] else None,
            ),
        )
        conn.commit()
        print(f"[STREAM] Thought inserted: {data['stream_id']} from {data['origin']}")
        return True
    except Exception as e:
        conn.rollback()
        print(f"[STREAM] Insert failed: {e}")
        return False
    finally:
        conn.close()
    return False


def _get_stream_context(self, limit: int = 20, hours: int = 24) -> list:
    """
    Get recent thoughts for agent context loading.
    Returns list of Thought dicts, most recent first.
    """
    conn = self.get_connection()
    cursor = conn.cursor()
    try:
        cutoff = (
            datetime.datetime.now(datetime.timezone.utc)
            - datetime.timedelta(hours=hours)
        ).isoformat()

        self._exec(
            cursor,
            """
            SELECT * FROM stream
            WHERE created_at > ?
            ORDER BY created_at DESC
            LIMIT ?
        """,
            (cutoff, limit),
        )
        rows = cursor.fetchall()
        results = []
        for row in rows:
            r = self._row_to_dict(row, cursor)
            # Parse JSON fields
            for field in ["artifact", "tags", "metadata"]:
                if r.get(field) and isinstance(r[field], str):
                    try:
                        r[field] = json.loads(r[field])
                    except:
                        pass
            results.append(r)
        return results
    finally:
        conn.close()


def _query_stream(
    self,
    origin: Optional[str] = None,
    intent_contains: Optional[str] = None,
    thought_type: Optional[str] = None,
    parent_id: Optional[str] = None,
    min_confidence: Optional[float] = None,
    limit: int = 50,
) -> List[Dict[str, Any]]:
    """
    Query the stream with filters.
    Used for debate chains, context lookup, and synthesis.
    """
    conn = self.get_connection()
    cursor = conn.cursor()
    try:
        query = "SELECT * FROM stream WHERE 1=1"
        params = []

        if origin:
            query += " AND origin = ?"
            params.append(origin)

        if intent_contains:
            query += " AND intent LIKE ?"
            params.append(f"%{intent_contains}%")

        if thought_type:
            query += " AND thought_type = ?"
            params.append(thought_type)

        if parent_id:
            query += " AND parent_id = ?"
            params.append(parent_id)

        if min_confidence is not None:
            query += " AND confidence >= ?"
            params.append(min_confidence)

        query += " ORDER BY created_at DESC LIMIT ?"
        params.append(limit)

        self._exec(cursor, query, tuple(params))
        rows = cursor.fetchall()
        results = []
        for row in rows:
            r = self._row_to_dict(row, cursor)
            for field in ["artifact", "tags", "metadata"]:
                if r.get(field) and isinstance(r[field], str):
                    try:
                        r[field] = json.loads(r[field])
                    except:
                        pass
            results.append(r)
        return results
    finally:
        conn.close()


def _get_debate_chain(self, root_stream_id: str) -> list:
    """
    Get all thoughts in a debate chain starting from root.
    Returns thoughts ordered by creation time (oldest first).
    """
    conn = self.get_connection()
    cursor = conn.cursor()
    try:
        # Get root thought
        self._exec(
            cursor, "SELECT * FROM stream WHERE stream_id = ?", (root_stream_id,)
        )
        root = cursor.fetchone()
        if not root:
            return []

        # Get all children recursively (simplified: one level deep for now)
        self._exec(
            cursor,
            """
            SELECT * FROM stream 
            WHERE parent_id = ? OR stream_id = ?
            ORDER BY created_at ASC
        """,
            (root_stream_id, root_stream_id),
        )
        rows = cursor.fetchall()
        results = []
        for row in rows:
            r = self._row_to_dict(row, cursor)
            for field in ["artifact", "tags", "metadata"]:
                if r.get(field) and isinstance(r[field], str):
                    try:
                        r[field] = json.loads(r[field])
                    except:
                        pass
            results.append(r)
        return results
    finally:
        conn.close()


# Monkey-patch methods onto Database class
Database.insert_thought = _insert_thought
Database.get_stream_context = _get_stream_context
Database.query_stream = _query_stream
Database.get_debate_chain = _get_debate_chain
