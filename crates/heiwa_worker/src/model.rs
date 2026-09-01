//! Worker and pane identity. I/O-free: this crate never spawns, reads, or
//! writes. The shell owns the process; this owns what the process *is*.

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

/// How a worker looks to the operator right now.
///
/// `Live` is a claim the runtime can defend: identity was appended, the process
/// started, and it reported a heartbeat. Everything weaker is named as weaker,
/// because the spec forbids treating a process as a verified worker merely
/// because it exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerState {
    Starting,
    Live,
    Stale,
    Exited,
    Failed,
    Revoked,
}

/// What a pane is showing. A pane bound to a worker that never reached `Live`
/// is `Unverified`, never `Working`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneState {
    Unverified,
    Working,
    Done,
    Failed,
    Stale,
}

impl PaneState {
    pub fn for_worker(worker: WorkerState) -> Self {
        match worker {
            WorkerState::Starting => PaneState::Unverified,
            WorkerState::Live => PaneState::Working,
            WorkerState::Exited => PaneState::Done,
            WorkerState::Failed | WorkerState::Revoked => PaneState::Failed,
            WorkerState::Stale => PaneState::Stale,
        }
    }
}

/// Everything a verified worker records, per the spec's "Legitimate Workers".
///
/// Deliberately absent: a parent worker, and the tool/filesystem/network/
/// budget/action lease split. A1-c2 has exactly one lease and no child
/// workers, so those fields would have nothing that could populate them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerIdentity {
    pub schema_version: u32,
    pub worker_id: String,
    pub work_id: String,
    pub thread_id: String,
    /// Provider name only. Never a token, cookie, or session secret.
    pub provider: String,
    /// Non-secret reference to a provider-owned session, when one exists.
    pub provider_session_ref: Option<String>,
    /// Absolute path of the executable that was actually opened.
    pub executable_path: String,
    /// SHA-256 of that file's bytes at launch time.
    pub executable_sha256: String,
    /// The prepared worktree. This is the execution location, not a hint.
    pub cwd: String,
    pub repo_root: String,
    pub branch: String,
    pub base_commit: String,
    /// The A1-b writer lease this worker executes under.
    pub lease_id: String,
    pub installation_id: String,
    pub started_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneIdentity {
    pub schema_version: u32,
    pub pane_id: String,
    pub work_id: String,
    pub worker_id: String,
    pub cwd: String,
    pub repo_root: String,
    pub branch: String,
    pub opened_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_worker_that_never_went_live_gives_an_unverified_pane() {
        assert_eq!(
            PaneState::for_worker(WorkerState::Starting),
            PaneState::Unverified
        );
        assert_eq!(PaneState::for_worker(WorkerState::Live), PaneState::Working);
        assert_eq!(PaneState::for_worker(WorkerState::Exited), PaneState::Done);
        assert_eq!(
            PaneState::for_worker(WorkerState::Revoked),
            PaneState::Failed
        );
        assert_eq!(PaneState::for_worker(WorkerState::Stale), PaneState::Stale);
    }

    #[test]
    fn worker_identity_round_trips() {
        let identity = WorkerIdentity {
            schema_version: SCHEMA_VERSION,
            worker_id: "worker-1".into(),
            work_id: "work-1".into(),
            thread_id: "thread-1".into(),
            provider: "claude".into(),
            provider_session_ref: None,
            executable_path: "/usr/local/bin/claude".into(),
            executable_sha256: "a".repeat(64),
            cwd: "/tmp/worktrees/work-1".into(),
            repo_root: "/tmp/repo".into(),
            branch: "heiwa/work-1".into(),
            base_commit: "b".repeat(40),
            lease_id: "lease-1".into(),
            installation_id: "install-1".into(),
            started_at: "2026-08-26T00:00:00Z".into(),
        };
        let encoded = serde_json::to_value(&identity).expect("encode");
        let decoded: WorkerIdentity = serde_json::from_value(encoded).expect("decode");
        assert_eq!(decoded, identity);
    }
}
