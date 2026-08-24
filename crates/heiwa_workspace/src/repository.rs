//! What a repository is, right now, as a recorded fact.
//!
//! Every field here is read-only. Nothing in this module writes to a
//! repository, so a snapshot can never be the thing that lost someone's work.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::git::{git, GitError};
use crate::WorkspaceError;

/// Version of the recorded repository shape.
pub const REPOSITORY_SNAPSHOT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositorySnapshotV1 {
    pub version: u32,
    /// The canonical path, with symlinks resolved.
    pub root: String,
    /// `None` on a detached HEAD, which is a state, not a failure.
    pub branch: Option<String>,
    pub head: String,
    pub remote: Option<String>,
    /// Whether anything is uncommitted, tracked or not.
    pub dirty: bool,
    /// Exactly which paths are uncommitted, so the fact is inspectable
    /// rather than a bare boolean the user has to trust.
    pub dirty_paths: Vec<String>,
}

/// Read `root` and record what it is.
pub fn snapshot_in(root: &Path) -> Result<RepositorySnapshotV1, WorkspaceError> {
    // Ask git for the repository root rather than trusting the argument: it
    // resolves symlinks and refuses a directory that only looks like a repo.
    let canonical = match git(root, &["rev-parse", "--show-toplevel"]) {
        Ok(path) => path,
        Err(GitError::Failed { .. }) => {
            return Err(WorkspaceError::NotARepository {
                path: root.display().to_string(),
            })
        }
        Err(other) => return Err(WorkspaceError::Git(other)),
    };

    let head = git(root, &["rev-parse", "HEAD"]).map_err(WorkspaceError::Git)?;

    // Empty output means a detached HEAD. Not an error, and not a branch.
    let branch = git(root, &["branch", "--show-current"]).map_err(WorkspaceError::Git)?;
    let branch = if branch.is_empty() {
        None
    } else {
        Some(branch)
    };

    // No origin is normal for a local repository, so a failure here is a fact
    // rather than a problem.
    let remote = git(root, &["remote", "get-url", "origin"])
        .ok()
        .filter(|url| !url.is_empty());

    let status = git(root, &["status", "--porcelain=v1"]).map_err(WorkspaceError::Git)?;
    let dirty_paths = parse_status_paths(&status);

    Ok(RepositorySnapshotV1 {
        version: REPOSITORY_SNAPSHOT_VERSION,
        root: canonical,
        branch,
        head,
        remote,
        dirty: !dirty_paths.is_empty(),
        dirty_paths,
    })
}

/// Pull the paths out of `git status --porcelain=v1`.
///
/// Each line is two status characters, a space, then the path — but a
/// modified tracked file is reported as `" M a.txt"` with a leading space,
/// and [`git`] trims its output, so the first line arrives one column short
/// of the rest. Splitting on the first whitespace run instead of slicing a
/// byte offset is correct either way.
///
/// A rename is `R  old -> new`; the new path is the one that exists, so that
/// is the one reported.
fn parse_status_paths(status: &str) -> Vec<String> {
    status
        .lines()
        .filter_map(|line| {
            let (_status, path) = line.trim_start().split_once(char::is_whitespace)?;
            let path = path.trim();
            if path.is_empty() {
                return None;
            }
            Some(match path.split_once(" -> ") {
                Some((_old, new)) => new.to_string(),
                None => path.to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::repo;

    #[test]
    fn a_clean_repository_reports_its_branch_head_and_cleanliness() {
        let dir = repo();
        let snapshot = snapshot_in(dir.path()).expect("snapshot");

        assert_eq!(snapshot.branch.as_deref(), Some("main"));
        assert_eq!(snapshot.head.len(), 40);
        assert!(!snapshot.dirty, "a fresh commit leaves nothing uncommitted");
        assert!(snapshot.dirty_paths.is_empty());
        assert_eq!(snapshot.remote, None, "a local repo has no origin");
    }

    #[test]
    fn an_uncommitted_change_is_recorded_as_a_visible_fact() {
        let dir = repo();
        std::fs::write(dir.path().join("a.txt"), "one\ntwo\n").expect("write");
        let snapshot = snapshot_in(dir.path()).expect("snapshot");

        assert!(snapshot.dirty);
        assert_eq!(
            snapshot.dirty_paths,
            vec!["a.txt".to_string()],
            "the user must be able to see exactly what is uncommitted"
        );
    }

    #[test]
    fn an_untracked_file_counts_as_dirty() {
        let dir = repo();
        std::fs::write(dir.path().join("new.txt"), "hello\n").expect("write");
        let snapshot = snapshot_in(dir.path()).expect("snapshot");

        assert!(snapshot.dirty, "untracked work is still the user's work");
        assert_eq!(snapshot.dirty_paths, vec!["new.txt".to_string()]);
    }

    #[test]
    fn a_detached_head_has_no_branch_and_is_not_an_error() {
        let dir = repo();
        let head = crate::git::git(dir.path(), &["rev-parse", "HEAD"]).expect("head");
        crate::git::git(dir.path(), &["checkout", "-q", "--detach", &head]).expect("detach");

        let snapshot = snapshot_in(dir.path()).expect("a detached head still snapshots");
        assert_eq!(snapshot.branch, None);
        assert_eq!(snapshot.head, head);
    }

    #[test]
    fn a_directory_that_is_not_a_repository_is_refused_by_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = snapshot_in(dir.path()).expect_err("not a repository");
        assert!(
            matches!(error, WorkspaceError::NotARepository { .. }),
            "the caller needs to distinguish this from a broken git: {error:?}"
        );
    }

    #[test]
    fn status_parsing_does_not_depend_on_column_offsets() {
        // The bug this locks in: `git()` trims its output, so the leading
        // space on " M a.txt" is gone by the time it is parsed while later
        // lines keep theirs. A fixed byte offset sliced into the filename and
        // produced ".txt".
        let paths = parse_status_paths(" M a.txt\n?? new.txt\nMM both.txt");
        assert_eq!(
            paths,
            vec![
                "a.txt".to_string(),
                "new.txt".to_string(),
                "both.txt".to_string()
            ]
        );

        let trimmed = parse_status_paths("M a.txt\n M b.txt");
        assert_eq!(
            trimmed,
            vec!["a.txt".to_string(), "b.txt".to_string()],
            "a trimmed first line must parse the same as an untrimmed one"
        );
    }

    #[test]
    fn a_rename_reports_the_path_that_now_exists() {
        let paths = parse_status_paths("R  old.txt -> new.txt");
        assert_eq!(paths, vec!["new.txt".to_string()]);
    }
}
