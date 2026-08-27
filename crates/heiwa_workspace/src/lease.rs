//! Exclusive permission for one Work to mutate one repository.
//!
//! Deliberately not a new store. `heiwa_evidence` already owns the
//! `worker_leases` stream, its replay, and `recover_interrupted`, which
//! revokes every lease left `issued` or `acked` when the runtime restarts. A
//! lease that outlived a crash would lock a repository against a Work that no
//! longer exists, and reusing the existing recovery is what stops that without
//! a second mechanism to remember.

use std::path::Path;

use heiwa_evidence::{EvidenceTransport, JsonlTransport, PersistedWorkerLease};
use serde::{Deserialize, Serialize};

use crate::WorkspaceError;

/// Statuses that mean a lease is still holding its resource.
const LIVE: [&str; 2] = ["issued", "acked"];

/// One Work's exclusive write hold on one repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriterLease {
    pub lease_id: String,
    pub work_id: String,
    /// The worker session this lease was issued for. A1-b had none and
    /// repeated `work_id` here; A1-c2 gives the lease a real worker, so a
    /// reader can tell which run held the repository.
    pub worker_id: String,
    /// `workspace.write:<canonical repository root>` — the resource the lease
    /// is exclusive over. Exclusivity is decided on this string.
    pub capability: String,
    /// Carried so releasing does not erase it. `upsert_worker_lease` appends,
    /// and replay keeps the last record per `lease_id`, so the release record
    /// *replaces* the issued one — anything not repeated here is destroyed.
    pub node_id: String,
    pub issued_at: String,
    pub expires_at: String,
}

fn capability_for(repo_root: &str) -> String {
    format!("workspace.write:{repo_root}")
}

/// Take the write lease on `repo_root` for `work_id`, or refuse and say who
/// holds it.
///
/// `evidence_dir` is where the journal lives; the caller resolves it, because
/// this crate resolves no roots.
#[allow(clippy::too_many_arguments)]
pub fn acquire_writer_lease(
    evidence_dir: &Path,
    transport: &JsonlTransport,
    work_id: &str,
    repo_root: &str,
    installation_id: &str,
    worker_id: &str,
    issued_at: &str,
    expires_at: &str,
    new_lease_id: impl FnOnce() -> String,
) -> Result<WriterLease, WorkspaceError> {
    let capability = capability_for(repo_root);
    if transport.dir() != evidence_dir {
        return Err(WorkspaceError::Evidence(format!(
            "lease transport root {} does not match replay root {}",
            transport.dir().display(),
            evidence_dir.display()
        )));
    }

    let lease_id = new_lease_id();
    let persisted = PersistedWorkerLease {
        lease_id: lease_id.clone(),
        task_id: work_id.to_string(),
        session_id: worker_id.to_string(),
        // No mesh node identity is required for local Work, exactly as
        // `Work.origin_node` stays `None` until enrolment.
        node_id: installation_id.to_string(),
        capability: capability.clone(),
        status: "issued".to_string(),
        issued_at: issued_at.to_string(),
        updated_at: issued_at.to_string(),
        expires_at: expires_at.to_string(),
        acked_at: None,
        completed_at: None,
        failure_code: None,
        reason: None,
    };

    if let Some(held) = transport
        .try_acquire_worker_lease(persisted)
        .map_err(|error| WorkspaceError::Evidence(error.to_string()))?
    {
        debug_assert!(LIVE.contains(&held.status.as_str()));
        return Err(WorkspaceError::LeaseHeld {
            repo_root: repo_root.to_string(),
            held_by: held.task_id.clone(),
        });
    }

    Ok(WriterLease {
        lease_id,
        work_id: work_id.to_string(),
        worker_id: worker_id.to_string(),
        capability,
        node_id: installation_id.to_string(),
        issued_at: issued_at.to_string(),
        expires_at: expires_at.to_string(),
    })
}

fn finish_writer_lease<T: EvidenceTransport>(
    transport: &T,
    lease: &WriterLease,
    finished_at: &str,
    status: &str,
    failure_code: Option<&str>,
    reason: &str,
) -> Result<(), WorkspaceError> {
    transport
        .upsert_worker_lease(PersistedWorkerLease {
            lease_id: lease.lease_id.clone(),
            task_id: lease.work_id.clone(),
            // Replay keeps only the last record per lease_id, so anything not
            // repeated here is destroyed. Carry the worker forward.
            session_id: lease.worker_id.clone(),
            node_id: lease.node_id.clone(),
            capability: lease.capability.clone(),
            status: status.to_string(),
            issued_at: lease.issued_at.clone(),
            updated_at: finished_at.to_string(),
            expires_at: lease.expires_at.clone(),
            acked_at: None,
            completed_at: Some(finished_at.to_string()),
            failure_code: failure_code.map(str::to_string),
            reason: Some(reason.to_string()),
        })
        .map_err(|error| WorkspaceError::Evidence(error.to_string()))
}

/// Give the repository back after successful use.
pub fn release_writer_lease<T: EvidenceTransport>(
    transport: &T,
    lease: &WriterLease,
    released_at: &str,
) -> Result<(), WorkspaceError> {
    finish_writer_lease(
        transport,
        lease,
        released_at,
        "completed",
        None,
        "workspace released",
    )
}

/// Give the repository back because preparation or execution failed.
pub fn revoke_writer_lease<T: EvidenceTransport>(
    transport: &T,
    lease: &WriterLease,
    revoked_at: &str,
    failure_code: &str,
    reason: &str,
) -> Result<(), WorkspaceError> {
    finish_writer_lease(
        transport,
        lease,
        revoked_at,
        "revoked",
        Some(failure_code),
        reason,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use heiwa_evidence::JsonlTransport;

    fn evidence() -> (tempfile::TempDir, JsonlTransport) {
        let dir = tempfile::tempdir().expect("tempdir");
        let transport = JsonlTransport::new(dir.path().to_path_buf()).expect("transport");
        (dir, transport)
    }

    #[test]
    fn acquiring_a_lease_records_it_as_issued() {
        let (dir, transport) = evidence();
        let lease = acquire_writer_lease(
            dir.path(),
            &transport,
            "work-abc",
            "/repo",
            "install-1",
            "worker-1",
            "2026-08-24T00:00:00Z",
            "2026-08-24T01:00:00Z",
            || "lease-1".to_string(),
        )
        .expect("acquire");

        assert_eq!(lease.lease_id, "lease-1");
        assert_eq!(lease.work_id, "work-abc");
        assert_eq!(lease.capability, "workspace.write:/repo");
    }

    #[test]
    fn a_second_work_cannot_hold_the_same_repository() {
        let (dir, transport) = evidence();
        acquire_writer_lease(
            dir.path(),
            &transport,
            "work-abc",
            "/repo",
            "install-1",
            "worker-1",
            "2026-08-24T00:00:00Z",
            "2026-08-24T01:00:00Z",
            || "lease-1".to_string(),
        )
        .expect("first acquire");

        let error = acquire_writer_lease(
            dir.path(),
            &transport,
            "work-def",
            "/repo",
            "install-1",
            "worker-1",
            "2026-08-24T00:00:00Z",
            "2026-08-24T01:00:00Z",
            || "lease-2".to_string(),
        )
        .expect_err("two writers on one repository is the thing this prevents");

        let WorkspaceError::LeaseHeld { held_by, .. } = &error else {
            panic!("expected LeaseHeld, got {error:?}");
        };
        assert_eq!(held_by, "work-abc", "the refusal must name the holder");
    }

    #[test]
    fn a_different_repository_is_not_blocked_by_an_unrelated_lease() {
        let (dir, transport) = evidence();
        acquire_writer_lease(
            dir.path(),
            &transport,
            "work-abc",
            "/repo-one",
            "install-1",
            "worker-1",
            "2026-08-24T00:00:00Z",
            "2026-08-24T01:00:00Z",
            || "lease-1".to_string(),
        )
        .expect("first");

        acquire_writer_lease(
            dir.path(),
            &transport,
            "work-def",
            "/repo-two",
            "install-1",
            "worker-1",
            "2026-08-24T00:00:00Z",
            "2026-08-24T01:00:00Z",
            || "lease-2".to_string(),
        )
        .expect("a lease is per repository, not global");
    }

    #[test]
    fn releasing_a_lease_frees_the_repository() {
        let (dir, transport) = evidence();
        let lease = acquire_writer_lease(
            dir.path(),
            &transport,
            "work-abc",
            "/repo",
            "install-1",
            "worker-1",
            "2026-08-24T00:00:00Z",
            "2026-08-24T01:00:00Z",
            || "lease-1".to_string(),
        )
        .expect("acquire");

        release_writer_lease(&transport, &lease, "2026-08-24T00:30:00Z").expect("release");

        acquire_writer_lease(
            dir.path(),
            &transport,
            "work-def",
            "/repo",
            "install-1",
            "worker-1",
            "2026-08-24T00:31:00Z",
            "2026-08-24T01:31:00Z",
            || "lease-2".to_string(),
        )
        .expect("a released repository is available again");
    }

    #[test]
    fn releasing_preserves_the_facts_the_issued_record_carried() {
        // upsert appends and replay keeps the last record, so a release that
        // omits a field silently destroys it.
        let (dir, transport) = evidence();
        let lease = acquire_writer_lease(
            dir.path(),
            &transport,
            "work-abc",
            "/repo",
            "install-1",
            "worker-1",
            "2026-08-24T00:00:00Z",
            "2026-08-24T01:00:00Z",
            || "lease-1".to_string(),
        )
        .expect("acquire");
        release_writer_lease(&transport, &lease, "2026-08-24T00:30:00Z").expect("release");

        let view = heiwa_evidence::WorkerStateView::replay(dir.path()).expect("replay");
        let stored = view.leases.get("lease-1").expect("lease survives replay");
        assert_eq!(stored.status, "completed");
        assert_eq!(stored.node_id, "install-1", "the node must survive release");
        assert_eq!(
            stored.issued_at, "2026-08-24T00:00:00Z",
            "release must not rewrite when the lease was issued"
        );
        assert_eq!(
            stored.expires_at, "2026-08-24T01:00:00Z",
            "release must preserve the original expiry fact"
        );
    }

    #[test]
    fn a_lease_does_not_survive_a_runtime_restart() {
        // The whole reason for reusing worker_leases: recovery already exists
        // and already does this. A crash must not leave a repository locked
        // forever by a Work that is no longer running.
        let (dir, transport) = evidence();
        acquire_writer_lease(
            dir.path(),
            &transport,
            "work-abc",
            "/repo",
            "install-1",
            "worker-1",
            "2026-08-24T00:00:00Z",
            "2026-08-24T01:00:00Z",
            || "lease-1".to_string(),
        )
        .expect("acquire");

        let report = heiwa_evidence::recover_interrupted(dir.path(), &transport).expect("recover");
        assert_eq!(report.leases_revoked, 1);

        acquire_writer_lease(
            dir.path(),
            &transport,
            "work-def",
            "/repo",
            "install-1",
            "worker-1",
            "2026-08-24T02:00:00Z",
            "2026-08-24T03:00:00Z",
            || "lease-2".to_string(),
        )
        .expect("restart recovery must free the repository");
    }
}
