//! Isolated worktrees: where a Work is allowed to change files.
//!
//! A worktree is built from a commit, which is what makes it safe. The user's
//! working tree is never read, written, stashed, or checked out by anything in
//! this module, so uncommitted work cannot be lost by creating one.
//!
//! The naming convention carries the `work_id`, so the owner of a directory is
//! visible from `git worktree list` alone rather than needing a side table
//! that can drift from the filesystem.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::git::git;
use crate::scope::resolve_in_scope;
use crate::WorkspaceError;

const BRANCH_PREFIX: &str = "heiwa/";

/// One Work's isolated checkout of one repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeHandle {
    pub work_id: String,
    /// Canonical path of the worktree directory.
    pub path: String,
    /// The branch the worktree is on, always `heiwa/<work_id>`.
    pub branch: String,
    /// The commit the worktree was created from.
    pub base_commit: String,
}

/// One entry from `git worktree list`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedWorktree {
    pub path: String,
    pub head: String,
    pub branch: Option<String>,
    /// Present when the branch follows the Heiwa naming convention.
    pub work_id: Option<String>,
}

fn branch_for(work_id: &str) -> String {
    format!("{BRANCH_PREFIX}{work_id}")
}

/// Create an isolated worktree for `work_id`, under `holding_dir`.
///
/// `holding_dir` is supplied by the caller because this crate does not resolve
/// roots; the shell passes a directory under the runtime root.
pub fn create_worktree_in(
    repo_root: &Path,
    holding_dir: &Path,
    work_id: &str,
) -> Result<WorktreeHandle, WorkspaceError> {
    if heiwa_work::WorkId::parse(work_id).is_none() {
        return Err(WorkspaceError::InvalidWorkId {
            work_id: work_id.to_string(),
        });
    }
    let branch = branch_for(work_id);

    if list_worktrees_in(repo_root)?
        .iter()
        .any(|listed| listed.work_id.as_deref() == Some(work_id))
    {
        return Err(WorkspaceError::WorktreeExists {
            work_id: work_id.to_string(),
        });
    }

    let target = holding_dir.join(work_id);
    let target = resolve_in_scope(holding_dir, &target)?;

    let base_commit = git(repo_root, &["rev-parse", "HEAD"])?;
    let target_str = target.to_string_lossy().to_string();

    git(
        repo_root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            &branch,
            &target_str,
            &base_commit,
        ],
    )?;

    Ok(WorktreeHandle {
        work_id: work_id.to_string(),
        path: target_str,
        branch,
        base_commit,
    })
}

/// Every worktree git knows about for this repository.
pub fn list_worktrees_in(repo_root: &Path) -> Result<Vec<ListedWorktree>, WorkspaceError> {
    let raw = git(repo_root, &["worktree", "list", "--porcelain"])?;
    Ok(parse_worktree_list(&raw))
}

/// Remove a worktree and its branch.
pub fn remove_worktree_in(repo_root: &Path, handle: &WorktreeHandle) -> Result<(), WorkspaceError> {
    git(repo_root, &["worktree", "remove", "--force", &handle.path])?;
    // The branch outlives the directory otherwise, and the next Work with the
    // same id would collide with a ref nothing points at. A deletion git
    // refuses is therefore incomplete cleanup, not a detail: reporting it as
    // success hands back a repository that will reject the next preparation
    // for this Work with no record of why.
    git(repo_root, &["branch", "-D", &handle.branch])?;
    Ok(())
}

/// Parse `git worktree list --porcelain`: blank-line separated blocks of
/// `worktree <path>`, `HEAD <sha>`, `branch <ref>`.
fn parse_worktree_list(raw: &str) -> Vec<ListedWorktree> {
    let mut out = Vec::new();
    let mut path: Option<String> = None;
    let mut head = String::new();
    let mut branch: Option<String> = None;

    fn flush(
        out: &mut Vec<ListedWorktree>,
        path: &mut Option<String>,
        head: &mut String,
        branch: &mut Option<String>,
    ) {
        if let Some(path_value) = path.take() {
            let work_id = branch
                .as_deref()
                .filter(|reference| reference.contains(BRANCH_PREFIX))
                .and_then(|reference| reference.rsplit('/').next().map(str::to_string));
            out.push(ListedWorktree {
                path: path_value,
                head: std::mem::take(head),
                branch: branch.take(),
                work_id,
            });
        }
    }

    for line in raw.lines() {
        if line.is_empty() {
            flush(&mut out, &mut path, &mut head, &mut branch);
        } else if let Some(rest) = line.strip_prefix("worktree ") {
            flush(&mut out, &mut path, &mut head, &mut branch);
            path = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("HEAD ") {
            head = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("branch ") {
            branch = Some(rest.to_string());
        }
    }
    flush(&mut out, &mut path, &mut head, &mut branch);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::repo;

    #[test]
    fn a_worktree_is_created_at_the_repositorys_head() {
        let source = repo();
        let holding = tempfile::tempdir().expect("holding");
        let head = crate::git::git(source.path(), &["rev-parse", "HEAD"]).expect("head");

        let handle =
            create_worktree_in(source.path(), holding.path(), "work-abc").expect("create worktree");

        assert_eq!(handle.base_commit, head);
        assert_eq!(handle.work_id, "work-abc");
        assert!(
            handle.branch.starts_with("heiwa/work-"),
            "{}",
            handle.branch
        );
        assert!(
            std::path::Path::new(&handle.path).join("a.txt").exists(),
            "the worktree must actually contain the tree"
        );
    }

    #[test]
    fn a_worktree_never_disturbs_uncommitted_work_in_the_source() {
        // Verified against real git while planning: `worktree add` builds from
        // a commit and leaves the source working tree alone. This test is what
        // keeps that true.
        let source = repo();
        let holding = tempfile::tempdir().expect("holding");
        std::fs::write(source.path().join("a.txt"), "one\nMINE\n").expect("dirty it");

        let handle =
            create_worktree_in(source.path(), holding.path(), "work-abc").expect("create worktree");

        let still_there = std::fs::read_to_string(source.path().join("a.txt")).expect("read");
        assert_eq!(
            still_there, "one\nMINE\n",
            "the user's uncommitted edit must survive untouched"
        );
        let in_worktree = std::fs::read_to_string(std::path::Path::new(&handle.path).join("a.txt"))
            .expect("read");
        assert_eq!(
            in_worktree, "one\n",
            "the worktree gets the committed state, not the dirty edit"
        );
    }

    #[test]
    fn two_works_get_two_separate_worktrees() {
        let source = repo();
        let holding = tempfile::tempdir().expect("holding");

        let first = create_worktree_in(source.path(), holding.path(), "work-abc").expect("first");
        let second = create_worktree_in(source.path(), holding.path(), "work-def").expect("second");

        assert_ne!(first.path, second.path);
        assert_ne!(first.branch, second.branch);
    }

    #[test]
    fn creating_a_second_worktree_for_one_work_is_refused() {
        let source = repo();
        let holding = tempfile::tempdir().expect("holding");
        create_worktree_in(source.path(), holding.path(), "work-abc").expect("first");

        let error = create_worktree_in(source.path(), holding.path(), "work-abc")
            .expect_err("one Work holds one worktree per repository");
        assert!(
            matches!(error, WorkspaceError::WorktreeExists { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn a_worktree_refuses_an_identity_that_is_not_one_safe_component() {
        let source = repo();
        let holding = tempfile::tempdir().expect("holding");

        let error = create_worktree_in(source.path(), holding.path(), "work-nested/value")
            .expect_err("Work identity becomes both a path and a git ref component");

        assert!(
            matches!(error, WorkspaceError::InvalidWorkId { .. }),
            "{error:?}"
        );
        assert!(
            !holding.path().join("work-nested").exists(),
            "refusal happens before filesystem mutation"
        );
    }

    #[test]
    fn a_created_worktree_is_listed_against_its_work() {
        let source = repo();
        let holding = tempfile::tempdir().expect("holding");
        create_worktree_in(source.path(), holding.path(), "work-abc").expect("create");

        let listed = list_worktrees_in(source.path()).expect("list");
        assert!(
            listed
                .iter()
                .any(|w| w.work_id.as_deref() == Some("work-abc")),
            "{listed:?}"
        );
    }

    #[test]
    fn removing_a_worktree_leaves_the_source_repository_intact() {
        let source = repo();
        let holding = tempfile::tempdir().expect("holding");
        let handle = create_worktree_in(source.path(), holding.path(), "work-abc").expect("create");

        remove_worktree_in(source.path(), &handle).expect("remove");

        assert!(!std::path::Path::new(&handle.path).exists());
        assert!(
            source.path().join("a.txt").exists(),
            "removing a worktree must never touch the source"
        );
    }

    #[test]
    fn a_refused_branch_deletion_reaches_the_caller() {
        let source = repo();
        let holding = tempfile::tempdir().expect("holding");
        let handle = create_worktree_in(source.path(), holding.path(), "work-abc").expect("create");

        // Reject deletion of this Work's branch and nothing else, so the only
        // thing under test is what removal does when git says no.
        let hooks = source.path().join(".git").join("hooks");
        std::fs::create_dir_all(&hooks).expect("hooks dir");
        let hook = hooks.join("reference-transaction");
        std::fs::write(
            &hook,
            "#!/bin/sh\nwhile read -r old new ref; do\n  case \"$ref\" in\n    refs/heads/heiwa/*) exit 1 ;;\n  esac\ndone\nexit 0\n",
        )
        .expect("write hook");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755))
                .expect("make the hook executable");
        }

        let error = remove_worktree_in(source.path(), &handle)
            .expect_err("a branch git would not delete is incomplete cleanup, not success");
        assert!(matches!(error, WorkspaceError::Git(_)), "{error:?}");
        assert!(
            crate::git::git(
                source.path(),
                &["rev-parse", "--verify", "--quiet", &handle.branch]
            )
            .is_ok(),
            "the stale branch is exactly what the caller now knows about"
        );
    }
}
