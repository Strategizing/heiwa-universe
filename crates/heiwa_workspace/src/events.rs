//! Workspace facts as operator-domain events.
//!
//! `Work` is a fold over the operator journal (A1-a). Workspace facts join the
//! same stream rather than a second store, so replaying one Work produces the
//! whole of it — including which repository it held and where it was allowed
//! to write.

use heiwa_evidence::{
    OperatorActor, OperatorEvent, OperatorEventType, OperatorRisk, OperatorSensitivity,
    OPERATOR_EVENT_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};

use crate::worktree::WorktreeHandle;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspacePreparedPayload {
    pub repo_root: String,
    pub worktree_path: String,
    pub branch: String,
    pub base_commit: String,
    /// The worker session this preparation reserved the repository for, and
    /// the lease it took. Optional because A1-b wrote neither, and events
    /// already on disk must keep deserializing unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worker_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceReleasedPayload {
    pub repo_root: String,
}

impl WorkspacePreparedPayload {
    pub fn from_event(event: &OperatorEvent) -> Option<Self> {
        if event.event_type != OperatorEventType::WorkspacePrepared {
            return None;
        }
        serde_json::from_value(event.payload.clone()).ok()
    }
}

impl WorkspaceReleasedPayload {
    pub fn from_event(event: &OperatorEvent) -> Option<Self> {
        if event.event_type != OperatorEventType::WorkspaceReleased {
            return None;
        }
        serde_json::from_value(event.payload.clone()).ok()
    }
}

fn scoped(
    work_id: &str,
    thread_id: &str,
    event_type: OperatorEventType,
    occurred_at: &str,
    payload: serde_json::Value,
    new_event_id: impl FnOnce() -> String,
) -> OperatorEvent {
    OperatorEvent {
        schema_version: OPERATOR_EVENT_SCHEMA_VERSION,
        event_id: new_event_id(),
        thread_id: thread_id.to_string(),
        turn_id: None,
        run_id: None,
        call_id: None,
        work_id: Some(work_id.to_string()),
        event_type,
        occurred_at: occurred_at.to_string(),
        actor: OperatorActor {
            kind: "user".to_string(),
            id: "local".to_string(),
        },
        risk_class: OperatorRisk::Low,
        sensitivity: OperatorSensitivity::LocalPrivate,
        parent_event_id: None,
        correlation_id: None,
        source_refs: Vec::new(),
        evidence_refs: Vec::new(),
        payload,
    }
}

pub fn workspace_prepared_event(
    work_id: &str,
    thread_id: &str,
    repo_root: &str,
    handle: &WorktreeHandle,
    worker_id: &str,
    lease_id: &str,
    occurred_at: &str,
    new_event_id: impl FnOnce() -> String,
) -> OperatorEvent {
    let payload = serde_json::json!({
        "repo_root": repo_root,
        "worktree_path": handle.path,
        "branch": handle.branch,
        "base_commit": handle.base_commit,
        "worker_id": worker_id,
        "lease_id": lease_id,
    });
    scoped(
        work_id,
        thread_id,
        OperatorEventType::WorkspacePrepared,
        occurred_at,
        payload,
        new_event_id,
    )
}

pub fn workspace_released_event(
    work_id: &str,
    thread_id: &str,
    repo_root: &str,
    occurred_at: &str,
    new_event_id: impl FnOnce() -> String,
) -> OperatorEvent {
    let payload = serde_json::json!({ "repo_root": repo_root });
    scoped(
        work_id,
        thread_id,
        OperatorEventType::WorkspaceReleased,
        occurred_at,
        payload,
        new_event_id,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handle() -> crate::worktree::WorktreeHandle {
        crate::worktree::WorktreeHandle {
            work_id: "work-abc".to_string(),
            path: "/holding/work-abc".to_string(),
            branch: "heiwa/work-abc".to_string(),
            base_commit: "abc123".to_string(),
        }
    }

    #[test]
    fn a_prepared_event_names_its_work_and_carries_the_worktree() {
        let event = workspace_prepared_event(
            "work-abc",
            "thread-1",
            "/repo",
            &handle(),
            "worker-1",
            "lease-1",
            "2026-08-24T00:00:00Z",
            || "evt-1".to_string(),
        );

        assert_eq!(event.work_id.as_deref(), Some("work-abc"));
        assert_eq!(event.event_type, OperatorEventType::WorkspacePrepared);

        let payload = WorkspacePreparedPayload::from_event(&event).expect("payload");
        assert_eq!(payload.repo_root, "/repo");
        assert_eq!(payload.worktree_path, "/holding/work-abc");
        assert_eq!(payload.base_commit, "abc123");
    }

    #[test]
    fn a_released_event_names_the_work_it_freed() {
        let event = workspace_released_event(
            "work-abc",
            "thread-1",
            "/repo",
            "2026-08-24T00:10:00Z",
            || "evt-2".to_string(),
        );

        assert_eq!(event.event_type, OperatorEventType::WorkspaceReleased);
        let payload = WorkspaceReleasedPayload::from_event(&event).expect("payload");
        assert_eq!(payload.repo_root, "/repo");
    }

    #[test]
    fn a_payload_from_the_wrong_event_type_is_refused() {
        let prepared = workspace_prepared_event(
            "work-abc",
            "thread-1",
            "/repo",
            &handle(),
            "worker-1",
            "lease-1",
            "2026-08-24T00:00:00Z",
            || "evt-1".to_string(),
        );
        assert!(WorkspaceReleasedPayload::from_event(&prepared).is_none());
    }
}
