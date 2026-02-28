from fastapi import FastAPI, HTTPException, Header, Depends
from pydantic import BaseModel
from typing import Optional, Dict, Any, List
import json
import yaml
import re
import os
import time
import threading
import datetime
from .db import db
from .config import settings
from .eligibility import compute_eligibility

from contextlib import asynccontextmanager


@asynccontextmanager
async def lifespan(app: FastAPI):
    start_alerts_scheduler()
    yield


app = FastAPI(title="Heiwa Hub", version="0.4.0", lifespan=lifespan)


class ProposalInput(BaseModel):
    proposal_id: str
    payload: Dict[str, Any]
    fingerprint: Optional[str] = None
    payload_raw: Optional[str] = None
    mode: str = "PRODUCTION"  # PRODUCTION or SIMULATION


class RunInput(BaseModel):
    run_id: str
    proposal_id: str
    status: str
    chain_result: Dict[str, Any]
    signals: Optional[List[Dict[str, Any]]] = None
    artifact_index: Optional[List[Dict[str, Any]]] = None
    node_instance_id: Optional[str] = None
    boot_ts: Optional[str] = None
    replay_receipt: Optional[Dict[str, Any]] = None
    mode: Optional[str] = None


class HeartbeatInput(BaseModel):
    node_id: str
    node_instance_id: str
    ts: str
    detail: Optional[Dict[str, Any]] = None


class NodeHeartbeatInput(BaseModel):
    meta: Optional[Dict[str, Any]] = None
    capabilities: Optional[Dict[str, Any]] = None
    agent_version: Optional[str] = None
    tags: Optional[List[str]] = None
    max_concurrency: Optional[int] = 1


class PreviewInput(BaseModel):
    work_type: str
    requirements: Dict[str, Any]


def verify_token(x_auth_token: str = Header(None)):
    if x_auth_token is None:
        raise HTTPException(status_code=401, detail="Missing Auth Token")
    if x_auth_token != settings.AUTH_TOKEN:
        raise HTTPException(status_code=403, detail="Invalid Auth Token")


def start_alerts_scheduler():
    enabled = os.getenv("ALERTS_SCAN_ENABLED", "0") == "1"
    if not enabled:
        return
    interval = int(os.getenv("ALERTS_SCAN_INTERVAL", "60"))

    def loop():
        while True:
            try:
                db.scan_lease_alerts(datetime.datetime.utcnow())
            except Exception as e:
                print(f"[ALERTS] scan failed: {e}")
            time.sleep(interval)

    t = threading.Thread(target=loop, daemon=True)
    t.start()


@app.get("/health")
def health():
    return {"status": "ok", "service": "heiwa_hub"}


@app.get("/health/ops", dependencies=[Depends(verify_token)])
def health_ops(limit: int = 20):
    """
    Operator status snapshot.
    Returns alerts summary and claimed proposals (TTL board).
    """
    return db.get_ops_snapshot(limit=limit)


@app.get("/whoami", dependencies=[Depends(verify_token)])
def whoami(x_auth_token: str = Header(None)):
    return {"authenticated": True, "mode": "operator_or_node", "service": "heiwa_hub"}


@app.post("/proposals", dependencies=[Depends(verify_token)])
def submit_proposal(proposal: ProposalInput):
    if not settings.PHASE2_WRITE_ENABLED:
        raise HTTPException(status_code=503, detail="Phase 2 write disabled")
    success = db.add_proposal(proposal.dict())
    if not success:
        raise HTTPException(status_code=409, detail="Proposal ID already exists")
    return {"status": "queued", "proposal_id": proposal.proposal_id}


class ClaimInput(BaseModel):
    node_id: str
    max_items: int = 1


@app.post("/proposals/claim", dependencies=[Depends(verify_token)])
def claim_proposals(claim: ClaimInput):
    """Atomic claim of assigned proposals for a node (Phase 2)."""
    if not settings.PHASE2_CLAIM_ENABLED:
        raise HTTPException(status_code=503, detail="Phase 2 claim disabled")

    claimed = db.claim_for_node(claim.node_id, claim.max_items)
    # Clean up payload for response
    for p in claimed:
        if "payload" in p:
            try:
                p["payload"] = json.loads(p["payload"])
            except:
                pass
    return {"claimed": claimed, "count": len(claimed)}


@app.get("/proposals/next", dependencies=[Depends(verify_token)])
def get_next_proposal(node_id: str):
    if not node_id:
        raise HTTPException(status_code=400, detail="node_id required")

    proposal = db.get_next_proposal(node_id)
    if not proposal:
        return {"proposal": None}

    # Parse payload if it's string (from DB)
    try:
        payload = json.loads(proposal["payload"])
    except:
        payload = proposal["payload"]

    return {
        "proposal": {
            "proposal_id": proposal["proposal_id"],
            "payload": payload,
            "created_at": proposal["created_at"],
            "status": proposal["status"],
            "node_id": proposal["node_id"],
            "claimed_at": proposal["claimed_at"],
            "lease_expires_at": proposal["lease_expires_at"],
        }
    }


@app.post("/runs", dependencies=[Depends(verify_token)])
def record_run(run: RunInput):
    # Optional size cap for replay_receipt metadata (64KB)
    if run.replay_receipt:
        import json as _json

        receipt_str = _json.dumps(run.replay_receipt)
        if len(receipt_str.encode("utf-8")) > 64 * 1024:
            raise HTTPException(status_code=400, detail="replay_receipt too large")

    success = db.record_run(run.dict())
    if not success:
        raise HTTPException(status_code=500, detail="Failed to record run")
    return {"status": "recorded", "run_id": run.run_id}


@app.get("/proposals", dependencies=[Depends(verify_token)])
def list_proposals(status: Optional[str] = None, limit: int = 50):
    proposals = db.get_proposals(status, limit)
    # Don't return full payload in list view to save BW
    for p in proposals:
        if "payload" in p:
            del p["payload"]
    return {"proposals": proposals}


@app.get("/proposals/{proposal_id}", dependencies=[Depends(verify_token)])
def get_proposal(proposal_id: str):
    proposal = db.get_proposal(proposal_id)
    if not proposal:
        raise HTTPException(status_code=404, detail="Proposal not found")

    try:
        proposal["payload"] = json.loads(proposal["payload"])
    except:
        pass
    return {"proposal": proposal}


@app.get("/runs", dependencies=[Depends(verify_token)])
def list_runs(proposal_id: Optional[str] = None, limit: int = 50):
    runs = db.get_runs(proposal_id, limit)
    # Clean up JSON fields
    for r in runs:
        for field in ["chain_result", "signals", "artifact_index", "replay_receipt"]:
            if r.get(field):
                try:
                    r[field] = json.loads(r[field])
                except:
                    pass
    return {"runs": runs}


@app.get("/runs/{run_id}", dependencies=[Depends(verify_token)])
def get_run(run_id: str):
    run = db.get_run(run_id)
    if not run:
        raise HTTPException(status_code=404, detail="Run not found")

    for field in ["chain_result", "signals", "artifact_index", "replay_receipt"]:
        if run.get(field):
            try:
                run[field] = json.loads(run[field])
            except:
                pass
    return {"run": run}


def _parse_json_if_needed(val):
    if val is None:
        return None
    if isinstance(val, (dict, list)):
        return val
    if isinstance(val, str):
        try:
            return json.loads(val)
        except Exception:
            return None
    return None


def _validate_artifact_index(index):
    allowed_purposes = {"LOG", "REPORT", "PATCH", "SCREENSHOT", "DATA"}
    if not isinstance(index, list):
        return False, "artifact_index is not a list", []

    sha_issues = []
    for idx, item in enumerate(index):
        if not isinstance(item, dict):
            return False, f"artifact_index[{idx}] is not an object", sha_issues
        for key in ["path", "purpose", "content_type", "bytes", "sha256"]:
            if key not in item:
                return False, f"artifact_index[{idx}] missing key {key}", sha_issues
        if item["purpose"] not in allowed_purposes:
            return False, f"artifact_index[{idx}] purpose invalid", sha_issues
        if not isinstance(item["bytes"], int) or item["bytes"] < 0:
            return False, f"artifact_index[{idx}] bytes invalid", sha_issues
        sha = item.get("sha256", "")
        if not re.fullmatch(r"[a-f0-9]{64}", str(sha)):
            sha_issues.append(f"artifact_index[{idx}] sha256 invalid")
    if sha_issues:
        return True, "sha issues", sha_issues
    return True, "ok", []


def _validate_receipt_schema(receipt):
    required = [
        "run_id",
        "proposal_id",
        "node_id",
        "node_instance_id",
        "boot_ts",
        "created_at",
        "policy_hash",
        "inputs_manifest_path",
        "execution_env_path",
        "replay_script_path",
    ]
    for key in required:
        if key not in receipt:
            return False, f"Missing required {key}"
        if not isinstance(receipt[key], str):
            return False, f"{key} must be string"
    if not re.fullmatch(r"[0-9a-fA-F-]{36}", receipt.get("node_instance_id", "")):
        return False, "node_instance_id format invalid"
    if not re.fullmatch(r"[a-f0-9]{64}", receipt.get("policy_hash", "")):
        return False, "policy_hash format invalid"
    return True, "ok"


def run_integrity_view(run):
    checks = []

    def add_check(cid, status, detail):
        checks.append({"id": cid, "status": status, "detail": detail})

    artifact_index = _parse_json_if_needed(run.get("artifact_index"))
    receipt_data = _parse_json_if_needed(run.get("replay_receipt"))

    # IDX_SCHEMA + SHA256_FORMAT
    if artifact_index is None:
        add_check("IDX_SCHEMA", "FAIL", "artifact_index missing or invalid JSON")
        add_check("SHA256_FORMAT", "FAIL", "artifact_index missing; cannot verify")
        paths = set()
    else:
        ok, detail, sha_issues = _validate_artifact_index(artifact_index)
        add_check("IDX_SCHEMA", "OK" if ok else "FAIL", detail)
        if sha_issues:
            add_check("SHA256_FORMAT", "FAIL", "; ".join(sha_issues))
        else:
            add_check("SHA256_FORMAT", "OK", "All sha256 values match expected format")
        paths = {item.get("path") for item in artifact_index if isinstance(item, dict)}

    # RECEIPT_PRESENT
    if "outputs/replay_receipt.json" in paths:
        add_check("RECEIPT_PRESENT", "OK", "Receipt path present in artifact_index")
    else:
        add_check(
            "RECEIPT_PRESENT",
            "WARN",
            "Receipt path missing from artifact_index",
        )

    # RECEIPT_SCHEMA + POLICY_HASH_FORMAT + RECEIPT_PATHS_IN_INDEX
    if receipt_data is None:
        add_check(
            "RECEIPT_SCHEMA",
            "WARN",
            "Receipt content not available in hub record",
        )
        add_check(
            "POLICY_HASH_FORMAT",
            "WARN",
            "Receipt content not available; cannot verify policy hash",
        )
        add_check(
            "RECEIPT_PATHS_IN_INDEX",
            "WARN",
            "Receipt content not available; cannot cross-check paths",
        )
    else:
        valid, detail = _validate_receipt_schema(receipt_data)
        add_check("RECEIPT_SCHEMA", "OK" if valid else "FAIL", detail)
        if re.fullmatch(r"[a-f0-9]{64}", receipt_data.get("policy_hash", "")):
            add_check("POLICY_HASH_FORMAT", "OK", "policy_hash is 64-hex")
        else:
            add_check("POLICY_HASH_FORMAT", "WARN", "policy_hash missing or invalid")

        receipt_paths = [
            receipt_data.get("inputs_manifest_path"),
            receipt_data.get("execution_env_path"),
            receipt_data.get("replay_script_path"),
            "outputs/replay_receipt.json",
        ]
        missing = [p for p in receipt_paths if p and p not in paths]
        if missing:
            add_check(
                "RECEIPT_PATHS_IN_INDEX",
                "WARN",
                f"Receipt paths missing from artifact_index: {missing}",
            )
        else:
            add_check(
                "RECEIPT_PATHS_IN_INDEX",
                "OK",
                "Receipt paths present in artifact_index",
            )

    overall = "OK"
    if any(c["status"] == "FAIL" for c in checks):
        overall = "FAIL"
    elif any(c["status"] == "WARN" for c in checks):
        overall = "WARN"

    return {"run_id": run.get("run_id"), "status": overall, "checks": checks}


@app.get("/runs/{run_id}/integrity", dependencies=[Depends(verify_token)])
def get_run_integrity(run_id: str):
    run = db.get_run(run_id)
    if not run:
        raise HTTPException(status_code=404, detail="Run not found")
    return run_integrity_view(run)


@app.get("/alerts", dependencies=[Depends(verify_token)])
def list_alerts(status: Optional[str] = "OPEN", limit: int = 50):
    alerts = db.get_alerts(status=status, limit=limit)
    return {"alerts": alerts}


@app.post("/alerts/{alert_id}/ack", dependencies=[Depends(verify_token)])
def ack_alert(alert_id: str):
    updated = db.update_alert_status(alert_id, "ACKED")
    if not updated:
        raise HTTPException(status_code=404, detail="Alert not found")
    return {"status": "acknowledged", "id": alert_id}


@app.post("/alerts/{alert_id}/close", dependencies=[Depends(verify_token)])
def close_alert(alert_id: str):
    updated = db.update_alert_status(alert_id, "CLOSED")
    if not updated:
        raise HTTPException(status_code=404, detail="Alert not found")
    return {"status": "closed", "id": alert_id}


@app.post("/alerts/scan", dependencies=[Depends(verify_token)])
def trigger_alert_scan():
    created_alerts = db.scan_alerts(datetime.datetime.utcnow())
    created_proposals = db.generate_proposals_from_alerts()
    return {
        "status": "scanned",
        "created_alerts": created_alerts,
        "created_proposals": created_proposals,
    }


@app.post("/proposals/{proposal_id}/heartbeat", dependencies=[Depends(verify_token)])
def proposal_heartbeat(proposal_id: str, heartbeat: HeartbeatInput):
    snap, err = db.update_heartbeat(
        proposal_id,
        heartbeat.node_id,
        heartbeat.node_instance_id,
        heartbeat.ts,
        heartbeat.detail,
    )
    if err == "NOT_FOUND":
        raise HTTPException(status_code=404, detail="Proposal not found")
    if err == "NOT_CLAIMED":
        raise HTTPException(status_code=409, detail="Proposal not claimed")
    if err == "NODE_MISMATCH":
        raise HTTPException(status_code=403, detail="Node mismatch")
    return {"proposal": snap}




@app.post("/nodes/{node_id}/heartbeat", dependencies=[Depends(verify_token)])
def node_heartbeat(node_id: str, hb: NodeHeartbeatInput):
    # Enforce size limit (approx check on body size via content-length header handled by nginx usually,
    # but here we can check fields).
    # We'll rely on fastAPI parsing, but maybe check serialized size if critical.
    # Directive says "Validate size < 4096 bytes".
    # Since we are using Pydantic, the payload is already parsed.
    # We can serialize back to check.
    payload_size = len(hb.json().encode("utf-8"))
    if payload_size > 4096:
        raise HTTPException(status_code=400, detail="Payload too large (>4096 bytes)")

    success = db.upsert_node_heartbeat(
        node_id, hb.meta, hb.capabilities, hb.agent_version, hb.tags, hb.max_concurrency
    )
    if not success:
        raise HTTPException(status_code=500, detail="Database write failed")
    return {"status": "heartbeat_received", "node_id": node_id}


@app.post("/assign/preview", dependencies=[Depends(verify_token)])
def assign_preview(preview: PreviewInput):
    nodes = db.list_nodes(status="ONLINE")  # Assignment likely only considers ONLINE?
    # Actually eligibility engine should filter status. Passing all might be better for "Ineligible because offline" feedback.
    # Let's pass all.
    all_nodes = db.list_nodes()

    # Parse json fields in main.py before passing to eligibility?
    # Eligibility engine handles raw dicts from DB?
    # db.list_nodes returns dicts but json fields are strings.
    # My eligibility.py implementation handled string parsing.

    result = compute_eligibility(all_nodes, preview.dict())
    return result


@app.get("/nodes", dependencies=[Depends(verify_token)])
def list_nodes(status: Optional[str] = None):
    nodes = db.list_nodes(status)
    # Parse meta_json
    for n in nodes:
        if n.get("meta_json"):
            try:
                n["meta"] = json.loads(n["meta_json"])
            except:
                n["meta"] = {}
        # Clean up internal fields
        if "meta_json" in n:
            del n["meta_json"]
    return {"nodes": nodes}


@app.get("/nodes/{node_id}", dependencies=[Depends(verify_token)])
def get_node(node_id: str):
    node = db.get_node(node_id)
    if not node:
        raise HTTPException(status_code=404, detail="Node not found")

    if node.get("meta_json"):
        try:
            node["meta"] = json.loads(node["meta_json"])
        except:
            node["meta"] = {}
    if "meta_json" in node:
        del node["meta_json"]

    return {"node": node}


# ========== PHASE 2: CONSENT API ==========


class ConsentInput(BaseModel):
    actor_type: str  # "human" or "system"
    actor_id: str
    decision: str  # "approve" or "reject"
    comment: Optional[str] = None


@app.post("/proposals/{proposal_id}/consent", dependencies=[Depends(verify_token)])
def submit_consent(proposal_id: str, consent: ConsentInput):
    """Append a consent record to a proposal."""
    proposal = db.get_proposal(proposal_id)
    if not proposal:
        raise HTTPException(status_code=404, detail="Proposal not found")

    # In Phase 2, we compute the hash here. For now, use a placeholder
    # since we don't have the full signing infrastructure yet.
    import hashlib

    payload_str = proposal.get("payload") or ""
    proposal_hash = hashlib.sha256(payload_str.encode("utf-8")).hexdigest()

    consent_id = db.append_consent(
        proposal_id,
        proposal_hash,
        consent.actor_type,
        consent.actor_id,
        consent.decision,
        consent.comment,
    )
    if not consent_id:
        raise HTTPException(status_code=500, detail="Failed to append consent")

    # If approved, transition proposal to APPROVED
    if consent.decision == "approve":
        now = datetime.datetime.now(datetime.timezone.utc)
        # Calculate expires_at from targeting if present
        ttl = 3600  # Default 1h
        try:
            targeting = json.loads(proposal.get("execution_targeting") or "{}")
            ttl = targeting.get("ttl_seconds", 3600)
        except:
            pass

        expires_at = (now + datetime.timedelta(seconds=ttl)).isoformat()
        db.transition_proposal_status(
            proposal_id,
            "APPROVED",
            {
                "approved_at": now.isoformat(),
                "expires_at": expires_at,
                "proposal_hash": proposal_hash,
            },
        )
    elif consent.decision == "reject":
        db.transition_proposal_status(proposal_id, "REJECTED")

    return {"status": "consent_recorded", "consent_id": consent_id}


@app.get("/proposals/{proposal_id}/consents", dependencies=[Depends(verify_token)])
def list_consents(proposal_id: str):
    """Get all consents for a proposal."""
    consents = db.get_consents_for_proposal(proposal_id)
    return {"consents": consents}