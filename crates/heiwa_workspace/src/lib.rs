//! The Workspace Coordinator: what a `Work` is allowed to touch on disk.
//!
//! One `Work` gets a recorded view of one repository, an isolated worktree to
//! mutate, and an exclusive writer lease. The crate shells out to `git`
//! through exactly one function (`git::git`); everything else folds that
//! output into typed facts.
//!
//! Root resolution is deliberately absent. `heiwa_config::HeiwaPaths` is the
//! only resolver in the product and `scripts/check_l0_acceptance.sh` fails on
//! a second one, so every entry point here takes a path.
//!
//! See `docs/superpowers/specs/2026-08-22-heiwa-work-fabric-design.md`.

pub mod events;
pub mod git;
pub mod lease;
pub mod projection;
pub mod repository;
pub mod scope;
pub mod worktree;

pub use events::{
    workspace_prepared_event, workspace_released_event, WorkspacePreparedPayload,
    WorkspaceReleasedPayload,
};
pub use git::{git, GitError};
pub use lease::{acquire_writer_lease, release_writer_lease, revoke_writer_lease, WriterLease};
pub use projection::{
    diff_projection_in, test_projection_in, ChangedFile, DiffProjectionV1, TestProjectionV1,
};
pub use repository::{snapshot_in, RepositorySnapshotV1, REPOSITORY_SNAPSHOT_VERSION};
pub use scope::resolve_in_scope;
pub use worktree::{
    create_worktree_in, list_worktrees_in, remove_worktree_in, ListedWorktree, WorktreeHandle,
};

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("{path} is not a git repository")]
    NotARepository { path: String },
    #[error(transparent)]
    Git(#[from] GitError),
    #[error("{path} resolves outside the permitted root {root}")]
    PathEscape { root: String, path: String },
    #[error("{work_id} already holds a worktree in this repository")]
    WorktreeExists { work_id: String },
    #[error("{work_id} is not a valid Work identity")]
    InvalidWorkId { work_id: String },
    #[error("{repo_root} is already held for writing by {held_by}")]
    LeaseHeld { repo_root: String, held_by: String },
    #[error("evidence journal error: {0}")]
    Evidence(String),
    // Not named `source`: thiserror reserves that for a nested Error, and
    // this is the io failure rendered as text.
    #[error("could not run verification command {command}: {reason}")]
    CommandUnavailable { command: String, reason: String },
}
