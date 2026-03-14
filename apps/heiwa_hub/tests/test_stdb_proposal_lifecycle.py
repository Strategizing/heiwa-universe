from __future__ import annotations

import datetime
import importlib
import json
import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))
sys.path.insert(0, str(ROOT / "packages/heiwa_protocol"))
sys.path.insert(0, str(ROOT / "packages/heiwa_identity"))
sys.path.insert(0, str(ROOT / "apps"))


class FakeSpacetimeDB:
    def __init__(self, *args, **kwargs):
        self.calls: list[tuple[str, object]] = []
        self.nodes: dict[str, dict] = {}
        self.proposals: dict[str, dict] = {}
        self.proposal_consents: dict[str, dict] = {}
        self.approval_requests: dict[str, dict] = {}
        self.approval_decisions: dict[str, dict] = {}
        self.capability_leases: dict[str, dict] = {}

    @staticmethod
    def _escape_sql_literal(value: str) -> str:
        return str(value).replace("'", "''")

    def add_proposal(self, proposal):
        row = dict(proposal)
        payload = row.get("payload")
        if isinstance(payload, (dict, list)):
            row["payload"] = json.dumps(payload)
        targeting = row.get("execution_targeting")
        if isinstance(targeting, (dict, list)):
            row["execution_targeting"] = json.dumps(targeting)
        eligibility = row.get("eligibility_snapshot")
        if isinstance(eligibility, (dict, list)):
            row["eligibility_snapshot"] = json.dumps(eligibility)
        row.setdefault("created_at", datetime.datetime.now(datetime.timezone.utc).isoformat())
        row.setdefault("status", "QUEUED")
        row.setdefault("mode", "PRODUCTION")
        row.setdefault("node_id", None)
        row.setdefault("claimed_at", None)
        row.setdefault("lease_expires_at", None)
        row.setdefault("last_heartbeat_at", None)
        row.setdefault("last_heartbeat_node_id", None)
        row.setdefault("last_heartbeat_node_instance_id", None)
        row.setdefault("last_heartbeat_detail", None)
        row.setdefault("assigned_node_id", None)
        row.setdefault("assignment_expires_at", None)
        row.setdefault("attempt_count", 0)
        row.setdefault("proposal_hash", None)
        row.setdefault("hub_signature", None)
        row.setdefault("approved_at", None)
        row.setdefault("expires_at", None)
        row.setdefault("eligibility_snapshot", None)
        self.proposals[row["proposal_id"]] = row
        self.calls.append(("add_proposal", row["proposal_id"]))
        return True

    def get_proposal(self, proposal_id):
        row = self.proposals.get(proposal_id)
        return dict(row) if row else None

    def get_proposals(self, status=None, limit=50):
        rows = list(self.proposals.values())
        if status:
            rows = [row for row in rows if row.get("status") == status]
        rows.sort(key=lambda row: row.get("created_at", ""), reverse=True)
        return [dict(row) for row in rows[:limit]]

    def get_routable_proposals(self):
        now_iso = datetime.datetime.now(datetime.timezone.utc).isoformat()
        rows = []
        for row in self.proposals.values():
            if row.get("status") not in {"APPROVED", "QUEUED"}:
                continue
            expires_at = row.get("expires_at")
            if expires_at and expires_at <= now_iso:
                continue
            rows.append(dict(row))
        rows.sort(key=lambda row: row.get("created_at", ""))
        return rows

    def list_nodes(self, status=None):
        rows = list(self.nodes.values())
        if status:
            rows = [row for row in rows if row.get("status") == status]
        rows.sort(key=lambda row: row["node_id"])
        return [dict(row) for row in rows]

    def assign_proposal(
        self,
        proposal_id,
        assigned_node_id,
        assignment_expires_at,
        hub_signature,
        proposal_hash,
        attempt_count,
        eligibility_snapshot=None,
    ):
        proposal = self.proposals.get(proposal_id)
        if not proposal:
            return False
        proposal["status"] = "ASSIGNED"
        proposal["assigned_node_id"] = assigned_node_id
        proposal["assignment_expires_at"] = assignment_expires_at
        proposal["hub_signature"] = hub_signature
        proposal["proposal_hash"] = proposal_hash
        proposal["attempt_count"] = attempt_count
        if eligibility_snapshot is not None:
            proposal["eligibility_snapshot"] = (
                json.dumps(eligibility_snapshot)
                if isinstance(eligibility_snapshot, (dict, list))
                else eligibility_snapshot
            )
        self.calls.append(("assign_proposal", proposal_id))
        return True

    def claim_proposal(self, proposal_id, node_id, claimed_at=None, lease_expires_at=None):
        proposal = self.proposals.get(proposal_id)
        if not proposal:
            return None
        if proposal.get("assigned_node_id") and proposal.get("assigned_node_id") != node_id:
            return None
        claimed_at = claimed_at or datetime.datetime.now(datetime.timezone.utc).isoformat()
        lease_expires_at = lease_expires_at or (
            datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(minutes=30)
        ).isoformat()
        proposal["status"] = "CLAIMED"
        proposal["node_id"] = node_id
        proposal["claimed_at"] = claimed_at
        proposal["lease_expires_at"] = lease_expires_at
        self.calls.append(("claim_proposal", proposal_id))
        return dict(proposal)

    def claim_next_approved_proposal(self, node_id):
        candidates = [
            row
            for row in self.proposals.values()
            if row.get("status") == "APPROVED"
        ]
        candidates.sort(key=lambda row: row.get("created_at", ""))
        if not candidates:
            return None
        return self.claim_proposal(candidates[0]["proposal_id"], node_id)

    def issue_capability_lease(self, lease_data):
        self.capability_leases[lease_data["lease_id"]] = dict(lease_data)
        self.calls.append(("issue_capability_lease", lease_data["lease_id"]))
        return True

    def get_active_capability_lease(self, proposal_id, holder_id):
        now_iso = datetime.datetime.now(datetime.timezone.utc).isoformat()
        rows = [
            row
            for row in self.capability_leases.values()
            if row.get("proposal_id") == proposal_id
            and row.get("holder_id") == holder_id
            and row.get("status") == "ACTIVE"
            and row.get("expires_at", now_iso) > now_iso
        ]
        rows.sort(key=lambda row: row.get("issued_at", ""), reverse=True)
        return dict(rows[0]) if rows else None

    def renew_capability_lease(self, lease_id, renewed_at, expires_at):
        lease = self.capability_leases.get(lease_id)
        if not lease:
            return False
        lease["renewed_at"] = renewed_at
        lease["expires_at"] = expires_at
        lease["status"] = "ACTIVE"
        self.calls.append(("renew_capability_lease", lease_id))
        return True

    def revoke_capability_lease(self, lease_id, revoked_at=None, revocation_reason=None):
        lease = self.capability_leases.get(lease_id)
        if not lease:
            return False
        lease["status"] = "REVOKED"
        lease["revoked_at"] = revoked_at
        lease["revocation_reason"] = revocation_reason
        self.calls.append(("revoke_capability_lease", lease_id))
        return True

    def record_proposal_heartbeat(self, proposal_id, node_id, node_instance_id, ts_iso, detail=None):
        proposal = self.proposals.get(proposal_id)
        if not proposal or proposal.get("node_id") != node_id:
            return None
        proposal["last_heartbeat_at"] = ts_iso
        proposal["last_heartbeat_node_id"] = node_id
        proposal["last_heartbeat_node_instance_id"] = node_instance_id
        proposal["last_heartbeat_detail"] = (
            json.dumps(detail) if isinstance(detail, (dict, list)) else detail
        )
        self.calls.append(("record_proposal_heartbeat", proposal_id))
        return dict(proposal)

    def record_consent(self, consent_data):
        proposal = self.proposals.get(consent_data["proposal_id"])
        if not proposal:
            return False
        request_id = consent_data.get("approval_request_id") or f"APR-{consent_data['proposal_id']}"
        self.approval_requests[request_id] = {
            "request_id": request_id,
            "proposal_id": consent_data["proposal_id"],
            "status": "APPROVED" if str(consent_data["decision"]).upper() == "APPROVE" else "REJECTED",
            "requested_at": consent_data.get("requested_at"),
            "expires_at": consent_data.get("request_expires_at"),
            "requested_by": consent_data.get("requested_by", "heiwa-hub"),
            "reason": consent_data.get("request_reason"),
            "payload_json": json.dumps(consent_data.get("request_payload", {})),
        }
        self.proposal_consents[consent_data["consent_id"]] = {
            "consent_id": consent_data["consent_id"],
            "proposal_id": consent_data["proposal_id"],
            "proposal_hash": consent_data.get("proposal_hash", "UNKNOWN"),
            "actor_type": consent_data["actor_type"],
            "actor_id": consent_data["actor_id"],
            "decision": str(consent_data["decision"]).upper(),
            "comment": consent_data.get("comment"),
            "created_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
            "metadata": json.dumps(consent_data.get("metadata", {})),
        }
        decision_id = f"DEC-{consent_data['consent_id']}"
        self.approval_decisions[decision_id] = {
            "decision_id": decision_id,
            "request_id": request_id,
            "proposal_id": consent_data["proposal_id"],
            "actor_type": consent_data["actor_type"],
            "actor_id": consent_data["actor_id"],
            "decision": str(consent_data["decision"]).upper(),
            "reason": consent_data.get("comment"),
            "created_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
            "metadata_json": json.dumps(consent_data.get("metadata", {})),
        }
        proposal["status"] = "APPROVED" if str(consent_data["decision"]).upper() == "APPROVE" else "REJECTED"
        proposal["proposal_hash"] = consent_data.get("proposal_hash")
        proposal["approved_at"] = consent_data.get("approved_at")
        proposal["expires_at"] = consent_data.get("expires_at")
        self.calls.append(("record_consent", consent_data["proposal_id"]))
        return True

    def get_consents_for_proposal(self, proposal_id):
        rows = [
            row for row in self.proposal_consents.values() if row.get("proposal_id") == proposal_id
        ]
        rows.sort(key=lambda row: row.get("created_at", ""))
        return [dict(row) for row in rows]

    def approve_proposal(self, proposal_id, proposal_hash, approved_at, expires_at=None):
        proposal = self.proposals.get(proposal_id)
        if not proposal:
            return False
        proposal["status"] = "APPROVED"
        proposal["proposal_hash"] = proposal_hash
        proposal["approved_at"] = approved_at
        proposal["expires_at"] = expires_at
        return True

    def reject_proposal(self, proposal_id):
        proposal = self.proposals.get(proposal_id)
        if not proposal:
            return False
        proposal["status"] = "REJECTED"
        return True

    def queue_proposal(self, proposal_id, eligibility_snapshot=None):
        proposal = self.proposals.get(proposal_id)
        if not proposal:
            return False
        proposal["status"] = "QUEUED"
        proposal["eligibility_snapshot"] = (
            json.dumps(eligibility_snapshot)
            if isinstance(eligibility_snapshot, (dict, list))
            else eligibility_snapshot
        )
        return True

    def expire_proposal(self, proposal_id, eligibility_snapshot=None):
        proposal = self.proposals.get(proposal_id)
        if not proposal:
            return False
        proposal["status"] = "EXPIRED"
        proposal["eligibility_snapshot"] = (
            json.dumps(eligibility_snapshot)
            if isinstance(eligibility_snapshot, (dict, list))
            else eligibility_snapshot
        )
        return True

    def requeue_proposal(self, proposal_id):
        proposal = self.proposals.get(proposal_id)
        if not proposal:
            return False
        proposal["status"] = "QUEUED"
        proposal["node_id"] = None
        proposal["claimed_at"] = None
        proposal["lease_expires_at"] = None
        proposal["assigned_node_id"] = None
        proposal["assignment_expires_at"] = None
        proposal["hub_signature"] = None
        return True

    def query(self, sql):
        if "FROM proposals" in sql and "status = 'ASSIGNED'" in sql:
            assigned_match = re.search(r"assigned_node_id = '([^']+)'", sql)
            limit_match = re.search(r"LIMIT (\d+)", sql)
            assigned_node_id = assigned_match.group(1) if assigned_match else None
            limit = int(limit_match.group(1)) if limit_match else 50
            rows = [
                dict(row)
                for row in self.proposals.values()
                if row.get("status") == "ASSIGNED"
                and row.get("assigned_node_id") == assigned_node_id
                and row.get("hub_signature")
            ]
            rows.sort(key=lambda row: row.get("created_at", ""))
            return rows[:limit]
        return []


def main() -> int:
    failures: list[str] = []

    original_backend = os.environ.get("HEIWA_STATE_BACKEND")
    original_identity = os.environ.get("STDB_IDENTITY")
    original_auth = os.environ.get("HEIWA_AUTH_TOKEN")
    os.environ["HEIWA_STATE_BACKEND"] = "spacetimedb"
    os.environ["STDB_IDENTITY"] = "heiwa_test_module"
    os.environ["HEIWA_AUTH_TOKEN"] = "heiwa-test-token"

    try:
        import heiwa_sdk.db as db_module

        original_stdb_class = db_module.SpacetimeDB
        original_db = db_module.db
        db_module.SpacetimeDB = FakeSpacetimeDB
        try:
            db = db_module.Database()
            db_module.db = db
            db.stdb.nodes["macbook@heiwa-node-a"] = {
                "node_id": "macbook@heiwa-node-a",
                "status": "ONLINE",
                "meta_json": json.dumps({"privilege_tier": "privileged_local"}),
                "capabilities_json": json.dumps(["shell", "build"]),
                "tags_json": json.dumps(["prod-approved"]),
                "agent_version": "2026.03.13",
                "max_concurrency": 4,
                "last_heartbeat_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
            }

            eligible = db.get_eligible_nodes(["shell"], "privileged_local")
            if len(eligible) != 1:
                failures.append(f"expected 1 eligible node, got {eligible}")

            proposal_1 = {
                "proposal_id": "prop-next-1",
                "status": "APPROVED",
                "payload": {"task": "direct-claim"},
                "execution_targeting": {
                    "requires": ["shell"],
                    "privilege_tier": "privileged_local",
                    "allowed_tools": ["heiwa_ops"],
                    "network_scope": {"mode": "deny"},
                    "filesystem_scope": {"allow": ["/Users/dmcgregsauce/heiwa"]},
                },
            }
            db.add_proposal(proposal_1)
            claimed_next = db.get_next_proposal("macbook@heiwa-node-a")
            if not claimed_next or claimed_next.get("status") != "CLAIMED":
                failures.append(f"get_next_proposal should claim an APPROVED proposal, got {claimed_next}")
            if not claimed_next or "lease_id" not in claimed_next:
                failures.append(f"get_next_proposal should issue a lease, got {claimed_next}")

            proposal_2 = {
                "proposal_id": "prop-assigned-1",
                "status": "APPROVED",
                "payload": {"task": "assigned-claim"},
                "execution_targeting": {
                    "requires": ["shell"],
                    "privilege_tier": "privileged_local",
                    "allowed_tools": ["heiwa_claw"],
                    "secret_scope": ["vault:openai"],
                },
            }
            db.add_proposal(proposal_2)
            assignment_expires = (
                datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(minutes=15)
            ).isoformat()
            if not db.assign_proposal_to_node(
                "prop-assigned-1",
                "macbook@heiwa-node-a",
                assignment_expires,
                proposal_hash="hash-assigned-1",
                hub_signature="SIG-hash-assigned-1",
                attempt_count=1,
                eligibility_snapshot={"eligible_count": 1},
            ):
                failures.append("assign_proposal_to_node should succeed on the STDB backend")
            claimed = db.claim_for_node("macbook@heiwa-node-a", max_items=1)
            if len(claimed) != 1 or claimed[0].get("proposal_id") != "prop-assigned-1":
                failures.append(f"claim_for_node should return the assigned proposal, got {claimed}")
            if claimed and "lease_id" not in claimed[0]:
                failures.append(f"claim_for_node should issue a capability lease, got {claimed}")

            heartbeat_ts = datetime.datetime.now(datetime.timezone.utc).isoformat()
            heartbeat, error = db.update_heartbeat(
                "prop-assigned-1",
                "macbook@heiwa-node-a",
                "instance-a",
                heartbeat_ts,
                {"cpu_pct": 11},
            )
            if error or not heartbeat or heartbeat.get("last_heartbeat_at") != heartbeat_ts:
                failures.append(f"update_heartbeat should succeed on claimed STDB proposals, got {heartbeat}, {error}")

            if not db.add_proposal(
                {
                    "proposal_id": "prop-consent-1",
                    "status": "QUEUED",
                    "payload": {"task": "approval"},
                    "execution_targeting": {"ttl_seconds": 600},
                }
            ):
                failures.append("failed to seed proposal for consent test")
            if not db.record_consent(
                {
                    "consent_id": "CON-approval-1",
                    "proposal_id": "prop-consent-1",
                    "proposal_hash": "hash-consent-1",
                    "actor_type": "human",
                    "actor_id": "operator",
                    "decision": "APPROVE",
                    "comment": "approved",
                    "metadata": {"source": "test"},
                    "requested_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                    "request_payload": {"task": "approval"},
                    "approved_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                    "expires_at": (
                        datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(minutes=10)
                    ).isoformat(),
                }
            ):
                failures.append("record_consent should succeed on the STDB backend")
            consented = db.get_proposal("prop-consent-1")
            if not consented or consented.get("status") != "APPROVED":
                failures.append(f"record_consent should approve the proposal, got {consented}")
            if len(db.stdb.approval_requests) != 1:
                failures.append(f"expected 1 approval request, got {db.stdb.approval_requests}")
            if len(db.stdb.approval_decisions) != 1:
                failures.append(f"expected 1 approval decision, got {db.stdb.approval_decisions}")
            if len(db.get_consents_for_proposal("prop-consent-1")) != 1:
                failures.append("expected one consent record after record_consent")

            import heiwa_sdk.main as main_module

            main_module = importlib.reload(main_module)
            main_module.db = db

            from fastapi.testclient import TestClient

            client = TestClient(main_module.app)
            create_response = client.post(
                "/proposals",
                headers={"x-auth-token": "heiwa-test-token"},
                json={
                    "proposal_id": "prop-http-create-1",
                    "payload": {"task": "http-create"},
                    "execution_targeting": {"ttl_seconds": 120, "requires": ["shell"]},
                },
            )
            if create_response.status_code != 200:
                failures.append(f"HTTP proposal create should return 200, got {create_response.status_code}: {create_response.text}")
            created = db.get_proposal("prop-http-create-1")
            if not created or "ttl_seconds" not in (created.get("execution_targeting") or ""):
                failures.append(f"HTTP proposal create should persist execution_targeting, got {created}")

            proposal_3 = {
                "proposal_id": "prop-http-1",
                "status": "QUEUED",
                "payload": {"task": "http-consent"},
                "execution_targeting": {"ttl_seconds": 300},
            }
            db.add_proposal(proposal_3)
            response = client.post(
                "/proposals/prop-http-1/consent",
                headers={"x-auth-token": "heiwa-test-token"},
                json={
                    "actor_type": "human",
                    "actor_id": "discord-user-1",
                    "decision": "approve",
                    "comment": "ship it",
                },
            )
            if response.status_code != 200:
                failures.append(f"HTTP consent endpoint should return 200, got {response.status_code}: {response.text}")
            http_proposal = db.get_proposal("prop-http-1")
            if not http_proposal or http_proposal.get("status") != "APPROVED":
                failures.append(f"HTTP consent endpoint should persist through STDB, got {http_proposal}")

            proposal_4 = {
                "proposal_id": "prop-http-claim-1",
                "status": "APPROVED",
                "payload": {"task": "http-claim"},
                "execution_targeting": {"requires": ["shell"], "privilege_tier": "privileged_local"},
            }
            db.add_proposal(proposal_4)
            db.assign_proposal_to_node(
                "prop-http-claim-1",
                "macbook@heiwa-node-a",
                (
                    datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(minutes=15)
                ).isoformat(),
                proposal_hash="hash-http-claim-1",
                hub_signature="SIG-http-claim-1",
                attempt_count=1,
            )
            claim_response = client.post(
                "/proposals/claim",
                headers={"x-auth-token": "heiwa-test-token"},
                json={"node_id": "macbook@heiwa-node-a", "max_items": 1},
            )
            claim_payload = claim_response.json()
            if claim_response.status_code != 200 or claim_payload.get("count") != 1:
                failures.append(f"HTTP claim endpoint should return one claimed proposal, got {claim_response.status_code}: {claim_payload}")
            elif "lease_token" not in claim_payload["claimed"][0]:
                failures.append(f"HTTP claim endpoint should expose lease_token for compatibility, got {claim_payload}")

            observed = {name for name, _ in db.stdb.calls}
            expected = {
                "add_proposal",
                "assign_proposal",
                "claim_proposal",
                "issue_capability_lease",
                "record_proposal_heartbeat",
                "renew_capability_lease",
                "record_consent",
            }
            missing = sorted(expected - observed)
            if missing:
                failures.append(f"missing STDB lifecycle calls: {', '.join(missing)}")
        finally:
            db_module.SpacetimeDB = original_stdb_class
            db_module.db = original_db
    finally:
        if original_backend is not None:
            os.environ["HEIWA_STATE_BACKEND"] = original_backend
        else:
            os.environ.pop("HEIWA_STATE_BACKEND", None)
        if original_identity is not None:
            os.environ["STDB_IDENTITY"] = original_identity
        else:
            os.environ.pop("STDB_IDENTITY", None)
        if original_auth is not None:
            os.environ["HEIWA_AUTH_TOKEN"] = original_auth
        else:
            os.environ.pop("HEIWA_AUTH_TOKEN", None)

    if failures:
        print("STDB proposal lifecycle test FAILED")
        for failure in failures:
            print(f" - {failure}")
        return 1

    print("STDB proposal lifecycle test PASSED")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
