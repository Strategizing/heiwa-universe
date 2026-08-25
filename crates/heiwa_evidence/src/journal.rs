//! Append-only JSONL journal transport.
//!
//! One stream per record kind (`<dir>/<kind>.jsonl`). Every line is a
//! versioned envelope `{v, at_ms, kind, record}`. Appends serialize through a
//! sidecar lock file (`.<kind>.jsonl.lock`) so multiple processes — runtime,
//! shell, orchestrator — can share one journal without torn lines, and so
//! compaction can atomically swap the stream without losing racing writes.

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::json;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;

use crate::records::*;
use crate::{journal_root, now_ms, EVIDENCE_SCHEMA_VERSION};

pub trait EvidenceTransport: Send + Sync + 'static {
    fn upsert_drex_decision(&self, decision: PersistedDrexDecision) -> Result<()>;
    fn insert_drex_failure(&self, failure: PersistedDrexFailure) -> Result<()>;
    fn attach_drex_decision_to_route(&self, request_id: &str, drex_decision_id: &str)
        -> Result<()>;
    fn upsert_worker_session(&self, session: PersistedWorkerSession) -> Result<()>;
    fn close_session(&self, session_id: String) -> Result<()>;
    fn upsert_worker_lease(&self, lease: PersistedWorkerLease) -> Result<()>;
    fn record_dispatch_ack(&self, ack: PersistedDispatchAck) -> Result<()>;
    fn register_artifact(&self, artifact: PersistedArtifact) -> Result<()>;
    fn record_run_receipt(&self, receipt: PersistedRunReceipt) -> Result<()>;
    fn record_run_failure(&self, failure: PersistedRunFailure) -> Result<()>;

    /// Untyped append for records that don't warrant a dedicated method
    /// (task dispatches, capability leases, node heartbeats, battlefields).
    fn journal(&self, _kind: &str, _payload: serde_json::Value) -> Result<()> {
        Ok(())
    }
}

/// Acquire the cross-process append lock for one stream. The lock file is a
/// stable sidecar: it survives compaction's rename of the data file, so a
/// writer can never end up appending to an unlinked inode.
pub(crate) fn lock_stream(dir: &Path, kind: &str) -> Result<File> {
    let lock_path = dir.join(format!(".{kind}.jsonl.lock"));
    // read+write, NOT append-only. `File::lock()` is `LockFileEx` on Windows,
    // which needs a handle carrying GENERIC_READ or GENERIC_WRITE. Opening
    // append-only yields FILE_APPEND_DATA, which is not sufficient, and the
    // lock fails with ERROR_ACCESS_DENIED (os error 5). On Unix `flock` is
    // happy with an append-only fd, so this only ever failed on Windows.
    //
    // truncate(false) because the lock file is a pure sentinel shared with
    // other processes; its length is irrelevant but clobbering it is not ours
    // to do.
    let lock_file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;
    lock_file.lock()?;
    Ok(lock_file)
}

pub(crate) fn stream_path(dir: &Path, kind: &str) -> PathBuf {
    dir.join(format!("{kind}.jsonl"))
}

/// Appends every record as one JSON line to `<dir>/<kind>.jsonl`.
#[derive(Debug)]
pub struct JsonlTransport {
    dir: PathBuf,
    write_lock: Mutex<()>,
}

impl JsonlTransport {
    pub fn new(dir: PathBuf) -> Result<Self> {
        std::fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            write_lock: Mutex::new(()),
        })
    }

    /// Default location: `~/.heiwa/evidence/`, overridable via
    /// `HEIWA_EVIDENCE_DIR` (tests and sandboxes must set it so they never
    /// write into the operator's real evidence corpus).
    pub fn default_local() -> Result<Self> {
        Self::new(journal_root()?)
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn encoded_record<T: Serialize>(kind: &str, record: &T) -> Result<Vec<u8>> {
        let line = json!({
            "v": EVIDENCE_SCHEMA_VERSION,
            "at_ms": now_ms(),
            "kind": kind,
            "record": record,
        })
        .to_string();
        let mut payload = line.into_bytes();
        payload.push(b'\n');
        Ok(payload)
    }

    fn append_locked<T: Serialize>(&self, kind: &str, record: &T) -> Result<()> {
        let payload = Self::encoded_record(kind, record)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(stream_path(&self.dir, kind))?;
        file.write_all(&payload)?;
        file.sync_data()?;
        Ok(())
    }

    fn append<T: Serialize>(&self, kind: &str, record: &T) -> Result<()> {
        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| anyhow!("evidence write lock poisoned"))?;
        let _stream_lock = lock_stream(&self.dir, kind)?;
        self.append_locked(kind, record)
    }

    /// Atomically acquire one worker capability.
    ///
    /// Returns `Ok(None)` when `lease` was appended. When another live lease
    /// already owns the same capability, returns that durable holder without
    /// appending anything. Replay and append share the stream lock, so two
    /// processes cannot both observe the capability as free.
    pub fn try_acquire_worker_lease(
        &self,
        lease: PersistedWorkerLease,
    ) -> Result<Option<PersistedWorkerLease>> {
        if !matches!(lease.status.as_str(), "issued" | "acked") {
            return Err(anyhow!(
                "worker lease acquisition requires issued or acked status, got {}",
                lease.status
            ));
        }
        let acquired_at = OffsetDateTime::parse(&lease.issued_at, &Rfc3339)
            .map_err(|error| anyhow!("invalid worker lease issued_at: {error}"))?;
        let candidate_expires_at = OffsetDateTime::parse(&lease.expires_at, &Rfc3339)
            .map_err(|error| anyhow!("invalid worker lease expires_at: {error}"))?;
        if candidate_expires_at <= acquired_at {
            return Err(anyhow!(
                "worker lease expires_at must be later than issued_at"
            ));
        }

        let _guard = self
            .write_lock
            .lock()
            .map_err(|_| anyhow!("evidence write lock poisoned"))?;
        let _stream_lock = lock_stream(&self.dir, "worker_leases")?;

        let replay = crate::replay::read_stream_unlocked(&self.dir, "worker_leases")?;
        if replay.skipped_lines > 0 {
            return Err(anyhow!(
                "worker lease stream is damaged ({} unreadable line(s)); refusing acquisition",
                replay.skipped_lines
            ));
        }
        let mut latest = HashMap::<String, PersistedWorkerLease>::new();
        for event in replay.events {
            if let Ok(persisted) = serde_json::from_value::<PersistedWorkerLease>(event.record) {
                latest.insert(persisted.lease_id.clone(), persisted);
            }
        }

        let mut expired = Vec::new();
        for candidate in latest.into_values().filter(|candidate| {
            candidate.capability == lease.capability
                && matches!(candidate.status.as_str(), "issued" | "acked")
        }) {
            let expires_at =
                OffsetDateTime::parse(&candidate.expires_at, &Rfc3339).map_err(|error| {
                    anyhow!(
                        "invalid expires_at on live worker lease {}: {error}",
                        candidate.lease_id
                    )
                })?;
            if expires_at > acquired_at {
                return Ok(Some(candidate));
            }
            expired.push(candidate);
        }

        for mut stale in expired {
            stale.status = "expired".to_string();
            stale.updated_at = lease.issued_at.clone();
            stale.completed_at = Some(lease.issued_at.clone());
            stale.failure_code = Some("LEASE_EXPIRED".to_string());
            stale.reason = Some("lease expired before successor acquisition".to_string());
            self.append_locked("worker_leases", &stale)?;
        }

        self.append_locked("worker_leases", &lease)?;
        Ok(None)
    }
}

impl EvidenceTransport for JsonlTransport {
    fn upsert_drex_decision(&self, decision: PersistedDrexDecision) -> Result<()> {
        self.append("drex_decisions", &decision)
    }

    fn insert_drex_failure(&self, failure: PersistedDrexFailure) -> Result<()> {
        self.append("drex_failures", &failure)
    }

    fn attach_drex_decision_to_route(
        &self,
        request_id: &str,
        drex_decision_id: &str,
    ) -> Result<()> {
        self.append(
            "route_links",
            &json!({ "request_id": request_id, "drex_decision_id": drex_decision_id }),
        )
    }

    fn upsert_worker_session(&self, session: PersistedWorkerSession) -> Result<()> {
        self.append("worker_sessions", &session)
    }

    fn close_session(&self, session_id: String) -> Result<()> {
        self.append("session_closes", &json!({ "session_id": session_id }))
    }

    fn upsert_worker_lease(&self, lease: PersistedWorkerLease) -> Result<()> {
        self.append("worker_leases", &lease)
    }

    fn record_dispatch_ack(&self, ack: PersistedDispatchAck) -> Result<()> {
        self.append("dispatch_acks", &ack)
    }

    fn register_artifact(&self, artifact: PersistedArtifact) -> Result<()> {
        self.append("artifacts", &artifact)
    }

    fn record_run_receipt(&self, receipt: PersistedRunReceipt) -> Result<()> {
        self.append("runs", &receipt)
    }

    fn record_run_failure(&self, failure: PersistedRunFailure) -> Result<()> {
        self.append("run_failures", &failure)
    }

    fn journal(&self, kind: &str, payload: serde_json::Value) -> Result<()> {
        self.append(kind, &payload)
    }
}

#[derive(Debug, Default, Clone, Copy)]
pub struct NoopTransport;

impl EvidenceTransport for NoopTransport {
    fn upsert_drex_decision(&self, _decision: PersistedDrexDecision) -> Result<()> {
        Ok(())
    }

    fn insert_drex_failure(&self, _failure: PersistedDrexFailure) -> Result<()> {
        Ok(())
    }

    fn attach_drex_decision_to_route(
        &self,
        _request_id: &str,
        _drex_decision_id: &str,
    ) -> Result<()> {
        Ok(())
    }

    fn upsert_worker_session(&self, _session: PersistedWorkerSession) -> Result<()> {
        Ok(())
    }

    fn close_session(&self, _session_id: String) -> Result<()> {
        Ok(())
    }

    fn upsert_worker_lease(&self, _lease: PersistedWorkerLease) -> Result<()> {
        Ok(())
    }

    fn record_dispatch_ack(&self, _ack: PersistedDispatchAck) -> Result<()> {
        Ok(())
    }

    fn register_artifact(&self, _artifact: PersistedArtifact) -> Result<()> {
        Ok(())
    }

    fn record_run_receipt(&self, _receipt: PersistedRunReceipt) -> Result<()> {
        Ok(())
    }

    fn record_run_failure(&self, _failure: PersistedRunFailure) -> Result<()> {
        Ok(())
    }
}
