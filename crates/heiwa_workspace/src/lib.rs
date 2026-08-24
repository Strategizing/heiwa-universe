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

pub mod git;
pub mod repository;

pub use git::{git, GitError};
pub use repository::{snapshot_in, RepositorySnapshotV1, REPOSITORY_SNAPSHOT_VERSION};

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("{path} is not a git repository")]
    NotARepository { path: String },
    #[error(transparent)]
    Git(#[from] GitError),
}
