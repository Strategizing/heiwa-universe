import datetime
import json
import logging
import subprocess
from typing import Any, List

logger = logging.getLogger("SDK.SpacetimeDB")


class SpacetimeDB:
    """
    Heiwa SpacetimeDB bridge.

    This remains CLI-backed for now, but the API surface is typed around the
    control-plane entities Heiwa treats as authoritative.
    """

    def __init__(self, db_identity: str, server: str = "maincloud"):
        self.db_identity = db_identity
        self.server = server

    @staticmethod
    def _escape_sql_literal(value: str) -> str:
        return str(value).replace("'", "''")

    @staticmethod
    def _json_text(value: Any) -> str:
        if value is None:
            return ""
        return json.dumps(value, separators=(",", ":"))

    @staticmethod
    def _normalize_json_column(value: Any) -> str | None:
        if value is None:
            return None
        if isinstance(value, str):
            return value
        return json.dumps(value, separators=(",", ":"))

    def _run(self, cmd: list[str], timeout: int = 10) -> subprocess.CompletedProcess[str] | None:
        try:
            result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        except Exception as exc:
            logger.error("STDB bridge error: %s", exc)
            return None

        if result.returncode != 0:
            logger.error("STDB command failed: %s", result.stderr.strip())
            return None
        return result

    def call(self, reducer_name: str, *args: Any) -> bool:
        cmd = ["spacetime", "call", "--server", self.server, self.db_identity, reducer_name]
        for arg in args:
            cmd.append(json.dumps(arg))

        result = self._run(cmd)
        if not result:
            return False
        logger.debug("STDB call %s succeeded: %s", reducer_name, result.stdout.strip())
        return True

    def query(self, sql: str) -> List[dict]:
        cmd = ["spacetime", "sql", "--server", self.server, "--json", self.db_identity, sql]
        result = self._run(cmd)
        if not result:
            return []

        try:
            payload = json.loads(result.stdout)
        except json.JSONDecodeError:
            return []

        if isinstance(payload, list):
            return payload
        if isinstance(payload, dict):
            rows = payload.get("rows") or payload.get("data")
            if isinstance(rows, list):
                return rows
            return [payload]
        return []

    def _first(self, sql: str) -> dict[str, Any] | None:
        rows = self.query(sql)
        return rows[0] if rows else None

    def record_route_decision(self, route: dict[str, Any]) -> bool:
        created_at = datetime.datetime.now(datetime.timezone.utc).isoformat()
        return self.call(
            "record_route_decision",
            route.get("request_id", ""),
            route.get("task_id", ""),
            route.get("envelope_version", ""),
            route.get("raw_text", ""),
            route.get("source_surface", "cli"),
            route.get("intent_class", "general"),
            route.get("risk_level", "low"),
            route.get("privacy_level", "local"),
            int(route.get("compute_class", 1)),
            route.get("assigned_worker", ""),
            route.get("target_tool", ""),
            route.get("target_model", ""),
            route.get("target_runtime", ""),
            route.get("target_tier", ""),
            bool(route.get("requires_approval", False)),
            route.get("rationale", ""),
            float(route.get("confidence", 0.0)),
            route.get("gateway_transport", "websocket"),
            created_at,
        )

    def record_run(self, run_data: dict[str, Any]) -> bool:
        ended_at = run_data.get("ended_at") or datetime.datetime.now(datetime.timezone.utc).isoformat()
        return self.call(
            "record_run",
            run_data.get("run_id", ""),
            run_data.get("proposal_id", ""),
            run_data.get("started_at") or "",
            ended_at,
            run_data.get("status", "UNKNOWN"),
            self._json_text(run_data.get("chain_result")),
            self._json_text(run_data.get("signals")),
            self._json_text(run_data.get("artifact_index")),
            run_data.get("node_id") or "",
            self._json_text(run_data.get("replay_receipt")),
            run_data.get("mode") or "",
            run_data.get("model_id") or "",
            int(run_data.get("tokens_input") or 0),
            int(run_data.get("tokens_output") or 0),
            int(run_data.get("tokens_total") or 0),
            float(run_data.get("cost") or 0.0),
        )

    def get_runs(self, proposal_id: str | None = None, limit: int = 50) -> list[dict[str, Any]]:
        query = (
            "SELECT run_id, proposal_id, started_at, ended_at, status, "
            "chain_result_json AS chain_result, signals_json AS signals, "
            "artifact_index_json AS artifact_index, node_id, replay_receipt_json AS replay_receipt, "
            "mode, model_id, tokens_input, tokens_output, tokens_total, cost "
            "FROM runs"
        )
        if proposal_id:
            query += f" WHERE proposal_id = '{self._escape_sql_literal(proposal_id)}'"
        query += f" ORDER BY ended_at DESC LIMIT {int(limit)}"
        return self.query(query)

    def get_model_usage_summary(self, minutes: int = 60) -> list[dict[str, Any]]:
        cutoff = (
            datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(minutes=minutes)
        ).isoformat()
        return self.query(
            "SELECT model_id, COUNT(*) AS request_count, "
            "SUM(tokens_total) AS total_tokens, SUM(cost) AS total_cost "
            f"FROM runs WHERE ended_at > '{cutoff}' AND model_id <> '' "
            "GROUP BY model_id"
        )

    def upsert_node_heartbeat(
        self,
        *,
        node_id: str,
        meta: dict[str, Any] | None = None,
        capabilities: dict[str, Any] | None = None,
        agent_version: str | None = None,
        tags: list[str] | None = None,
        max_concurrency: int = 1,
    ) -> bool:
        now_iso = datetime.datetime.now(datetime.timezone.utc).isoformat()
        return self.call(
            "upsert_node_heartbeat",
            node_id,
            now_iso,
            now_iso,
            self._json_text(meta or {}),
            self._json_text(capabilities or {}),
            agent_version or "",
            self._json_text(tags or []),
            int(max_concurrency),
        )

    def set_node_status(self, node_id: str, status: str) -> bool:
        return self.call("set_node_status", node_id, status)

    def list_nodes(self, status: str | None = None) -> list[dict[str, Any]]:
        query = "SELECT * FROM nodes"
        if status:
            query += f" WHERE status = '{self._escape_sql_literal(status)}'"
        query += " ORDER BY node_id"
        return self.query(query)

    def get_node(self, node_id: str) -> dict[str, Any] | None:
        return self._first(
            f"SELECT * FROM nodes WHERE node_id = '{self._escape_sql_literal(node_id)}' LIMIT 1"
        )

    def upsert_liveness_state(self, key: str, state: str, changed_at: str | None = None) -> bool:
        return self.call(
            "upsert_liveness_state",
            key,
            state,
            changed_at or datetime.datetime.now(datetime.timezone.utc).isoformat(),
        )

    def get_liveness_state(self, key: str) -> dict[str, Any] | None:
        return self._first(
            "SELECT key, last_state, last_changed_at FROM liveness_state "
            f"WHERE key = '{self._escape_sql_literal(key)}' LIMIT 1"
        )

    def add_proposal(self, proposal: dict[str, Any]) -> bool:
        payload = proposal.get("payload")
        payload_str = json.dumps(payload) if isinstance(payload, (dict, list)) else str(payload or "")
        return self.call(
            "add_proposal",
            proposal["proposal_id"],
            proposal.get("created_at") or datetime.datetime.now(datetime.timezone.utc).isoformat(),
            proposal.get("status", "QUEUED"),
            proposal.get("fingerprint"),
            payload_str,
            proposal.get("payload_raw"),
            proposal.get("mode", "PRODUCTION"),
            self._normalize_json_column(proposal.get("execution_targeting")),
            proposal.get("assigned_node_id"),
            proposal.get("hub_signature"),
            proposal.get("assignment_expires_at"),
            int(proposal.get("attempt_count") or 0),
            proposal.get("proposal_hash"),
            proposal.get("approved_at"),
            proposal.get("expires_at"),
            self._normalize_json_column(proposal.get("eligibility_snapshot")),
        )

    def get_proposals(self, status: str | None = None, limit: int = 50) -> list[dict[str, Any]]:
        query = "SELECT * FROM proposals"
        if status:
            query += f" WHERE status = '{self._escape_sql_literal(status)}'"
        query += f" ORDER BY created_at DESC LIMIT {int(limit)}"
        return self.query(query)

    def get_proposal(self, proposal_id: str) -> dict[str, Any] | None:
        return self._first(
            f"SELECT * FROM proposals WHERE proposal_id = '{self._escape_sql_literal(proposal_id)}' LIMIT 1"
        )

    def get_routable_proposals(self) -> list[dict[str, Any]]:
        now_iso = datetime.datetime.now(datetime.timezone.utc).isoformat()
        return self.query(
            "SELECT * FROM proposals "
            "WHERE status IN ('APPROVED', 'QUEUED') "
            f"AND (expires_at IS NULL OR expires_at = '' OR expires_at > '{self._escape_sql_literal(now_iso)}') "
            "ORDER BY created_at ASC"
        )

    def assign_proposal(
        self,
        proposal_id: str,
        assigned_node_id: str,
        assignment_expires_at: str,
        hub_signature: str,
        proposal_hash: str,
        attempt_count: int,
        eligibility_snapshot: dict[str, Any] | str | None = None,
    ) -> bool:
        return self.call(
            "assign_proposal",
            proposal_id,
            assigned_node_id,
            assignment_expires_at,
            hub_signature,
            proposal_hash,
            int(attempt_count),
            self._normalize_json_column(eligibility_snapshot),
        )

    def claim_proposal(
        self,
        proposal_id: str,
        node_id: str,
        claimed_at: str | None = None,
        lease_expires_at: str | None = None,
    ) -> dict[str, Any] | None:
        claimed_at = claimed_at or datetime.datetime.now(datetime.timezone.utc).isoformat()
        lease_expires_at = lease_expires_at or (
            datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(minutes=30)
        ).isoformat()
        if not self.call("claim_proposal", proposal_id, node_id, claimed_at, lease_expires_at):
            return None
        return self.get_proposal(proposal_id)

    def claim_next_approved_proposal(self, node_id: str) -> dict[str, Any] | None:
        now_iso = datetime.datetime.now(datetime.timezone.utc).isoformat()
        proposal = self._first(
            "SELECT * FROM proposals "
            "WHERE status = 'APPROVED' "
            f"AND (expires_at IS NULL OR expires_at = '' OR expires_at > '{self._escape_sql_literal(now_iso)}') "
            "ORDER BY created_at ASC LIMIT 1"
        )
        if not proposal:
            return None
        return self.claim_proposal(proposal["proposal_id"], node_id)

    def approve_proposal(
        self,
        proposal_id: str,
        proposal_hash: str,
        approved_at: str,
        expires_at: str | None = None,
    ) -> bool:
        return self.call("approve_proposal", proposal_id, approved_at, expires_at, proposal_hash)

    def reject_proposal(self, proposal_id: str) -> bool:
        return self.call("reject_proposal", proposal_id)

    def queue_proposal(self, proposal_id: str, eligibility_snapshot: dict[str, Any] | str | None = None) -> bool:
        return self.call("queue_proposal", proposal_id, self._normalize_json_column(eligibility_snapshot))

    def expire_proposal(self, proposal_id: str, eligibility_snapshot: dict[str, Any] | str | None = None) -> bool:
        return self.call("expire_proposal", proposal_id, self._normalize_json_column(eligibility_snapshot))

    def requeue_proposal(self, proposal_id: str) -> bool:
        return self.call("requeue_proposal", proposal_id)

    def record_proposal_heartbeat(
        self,
        proposal_id: str,
        node_id: str,
        node_instance_id: str,
        ts_iso: str,
        detail: dict[str, Any] | None = None,
    ) -> dict[str, Any] | None:
        if not self.call(
            "record_proposal_heartbeat",
            proposal_id,
            node_id,
            node_instance_id,
            ts_iso,
            self._normalize_json_column(detail),
        ):
            return None
        return self.get_proposal(proposal_id)

    def record_consent(self, consent_data: dict[str, Any]) -> bool:
        metadata = consent_data.get("metadata")
        request_payload = consent_data.get("request_payload")
        return self.call(
            "record_consent",
            consent_data["consent_id"],
            consent_data["proposal_id"],
            consent_data.get("proposal_hash", "UNKNOWN"),
            consent_data["actor_type"],
            consent_data["actor_id"],
            str(consent_data["decision"]).upper(),
            consent_data.get("comment"),
            self._normalize_json_column(metadata) or "{}",
            consent_data.get("approval_request_id"),
            consent_data.get("requested_by"),
            consent_data.get("requested_at"),
            consent_data.get("request_expires_at"),
            consent_data.get("request_reason"),
            self._normalize_json_column(request_payload),
            consent_data.get("approved_at"),
            consent_data.get("expires_at"),
        )

    def get_consents_for_proposal(self, proposal_id: str) -> list[dict[str, Any]]:
        return self.query(
            "SELECT * FROM proposal_consents "
            f"WHERE proposal_id = '{self._escape_sql_literal(proposal_id)}' ORDER BY created_at"
        )

    def add_approval_request(self, request_data: dict[str, Any]) -> bool:
        return self.call(
            "add_approval_request",
            request_data["request_id"],
            request_data["proposal_id"],
            request_data.get("status", "PENDING"),
            request_data.get("requested_at") or datetime.datetime.now(datetime.timezone.utc).isoformat(),
            request_data.get("expires_at"),
            request_data.get("requested_by", "heiwa-hub"),
            request_data.get("reason"),
            self._normalize_json_column(request_data.get("payload")) or "{}",
        )

    def record_approval_decision(self, decision_data: dict[str, Any]) -> bool:
        return self.call(
            "record_approval_decision",
            decision_data["decision_id"],
            decision_data["request_id"],
            decision_data["proposal_id"],
            decision_data["actor_type"],
            decision_data["actor_id"],
            str(decision_data["decision"]).upper(),
            decision_data.get("reason"),
            decision_data.get("created_at") or datetime.datetime.now(datetime.timezone.utc).isoformat(),
            self._normalize_json_column(decision_data.get("metadata")) or "{}",
        )

    def list_approval_requests(
        self,
        proposal_id: str | None = None,
        status: str | None = None,
        limit: int = 50,
    ) -> list[dict[str, Any]]:
        clauses: list[str] = []
        if proposal_id:
            clauses.append(f"proposal_id = '{self._escape_sql_literal(proposal_id)}'")
        if status:
            clauses.append(f"status = '{self._escape_sql_literal(status)}'")
        query = "SELECT * FROM approval_requests"
        if clauses:
            query += " WHERE " + " AND ".join(clauses)
        query += f" ORDER BY requested_at DESC LIMIT {int(limit)}"
        return self.query(query)

    def list_approval_decisions(
        self,
        proposal_id: str | None = None,
        request_id: str | None = None,
        limit: int = 50,
    ) -> list[dict[str, Any]]:
        clauses: list[str] = []
        if proposal_id:
            clauses.append(f"proposal_id = '{self._escape_sql_literal(proposal_id)}'")
        if request_id:
            clauses.append(f"request_id = '{self._escape_sql_literal(request_id)}'")
        query = "SELECT * FROM approval_decisions"
        if clauses:
            query += " WHERE " + " AND ".join(clauses)
        query += f" ORDER BY created_at DESC LIMIT {int(limit)}"
        return self.query(query)

    def issue_capability_lease(self, lease_data: dict[str, Any]) -> bool:
        return self.call(
            "issue_capability_lease",
            lease_data["lease_id"],
            lease_data["proposal_id"],
            lease_data.get("run_id"),
            lease_data.get("holder_kind", "node"),
            lease_data["holder_id"],
            self._normalize_json_column(lease_data.get("tool_scope")) or "[]",
            self._normalize_json_column(lease_data.get("network_scope")) or "{}",
            self._normalize_json_column(lease_data.get("filesystem_scope")) or "{}",
            self._normalize_json_column(lease_data.get("secret_scope")) or "[]",
            lease_data.get("privilege_tier", "cloud_safe"),
            lease_data.get("status", "ACTIVE"),
            lease_data.get("issued_at") or datetime.datetime.now(datetime.timezone.utc).isoformat(),
            lease_data["expires_at"],
            lease_data.get("hub_signature", ""),
        )

    def renew_capability_lease(self, lease_id: str, renewed_at: str, expires_at: str) -> bool:
        return self.call("renew_capability_lease", lease_id, renewed_at, expires_at)

    def revoke_capability_lease(
        self,
        lease_id: str,
        revoked_at: str | None = None,
        revocation_reason: str | None = None,
    ) -> bool:
        return self.call(
            "revoke_capability_lease",
            lease_id,
            revoked_at or datetime.datetime.now(datetime.timezone.utc).isoformat(),
            revocation_reason,
        )

    def get_capability_leases(
        self,
        proposal_id: str | None = None,
        holder_id: str | None = None,
        status: str | None = None,
        limit: int = 50,
    ) -> list[dict[str, Any]]:
        clauses: list[str] = []
        if proposal_id:
            clauses.append(f"proposal_id = '{self._escape_sql_literal(proposal_id)}'")
        if holder_id:
            clauses.append(f"holder_id = '{self._escape_sql_literal(holder_id)}'")
        if status:
            clauses.append(f"status = '{self._escape_sql_literal(status)}'")
        query = "SELECT * FROM capability_leases"
        if clauses:
            query += " WHERE " + " AND ".join(clauses)
        query += f" ORDER BY issued_at DESC LIMIT {int(limit)}"
        return self.query(query)

    def get_active_capability_lease(self, proposal_id: str, holder_id: str) -> dict[str, Any] | None:
        now_iso = datetime.datetime.now(datetime.timezone.utc).isoformat()
        return self._first(
            "SELECT * FROM capability_leases "
            f"WHERE proposal_id = '{self._escape_sql_literal(proposal_id)}' "
            f"AND holder_id = '{self._escape_sql_literal(holder_id)}' "
            "AND status = 'ACTIVE' "
            f"AND expires_at > '{self._escape_sql_literal(now_iso)}' "
            "ORDER BY issued_at DESC LIMIT 1"
        )

    def get_discord_channel(self, purpose: str) -> int | None:
        row = self._first(
            "SELECT channel_id FROM discord_channels "
            f"WHERE purpose = '{self._escape_sql_literal(purpose)}' LIMIT 1"
        )
        if row and "channel_id" in row:
            return int(row["channel_id"])
        return None

    def get_user_trust(self, user_id: int) -> float:
        row = self._first(
            f"SELECT trust_score FROM discord_users WHERE user_id = {int(user_id)} LIMIT 1"
        )
        if row and "trust_score" in row:
            return float(row["trust_score"])
        return 0.5
