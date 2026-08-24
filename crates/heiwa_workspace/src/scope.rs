//! Deciding whether a path is inside a root, after the filesystem has had its
//! say.
//!
//! A string prefix comparison is not this check. `root/link` can be a symlink
//! to anywhere on the disk, and it passes a prefix test right up until it is
//! opened. Both sides are canonicalised first so the comparison is between
//! real locations.

use std::path::{Path, PathBuf};

use crate::WorkspaceError;

/// Resolve `candidate` and confirm it lies within `root`.
///
/// `candidate` need not exist: a worktree is named before it is created. When
/// it does not exist, its nearest existing ancestor is canonicalised instead
/// and the remaining components are appended, so a new name is judged by where
/// it would actually land.
pub fn resolve_in_scope(root: &Path, candidate: &Path) -> Result<PathBuf, WorkspaceError> {
    let escape = || WorkspaceError::PathEscape {
        root: root.display().to_string(),
        path: candidate.display().to_string(),
    };

    let canonical_root = root.canonicalize().map_err(|_| escape())?;
    let resolved = canonicalize_existing_prefix(candidate).ok_or_else(escape)?;

    if resolved.starts_with(&canonical_root) {
        Ok(resolved)
    } else {
        Err(escape())
    }
}

/// Canonicalise as much of `path` as exists, then re-append the rest.
fn canonicalize_existing_prefix(path: &Path) -> Option<PathBuf> {
    if let Ok(resolved) = path.canonicalize() {
        return Some(resolved);
    }

    let mut trailing = Vec::new();
    let mut cursor = path;
    loop {
        let parent = cursor.parent()?;
        let name = cursor.file_name()?;
        trailing.push(name.to_os_string());
        if let Ok(resolved) = parent.canonicalize() {
            let mut out = resolved;
            for component in trailing.iter().rev() {
                out.push(component);
            }
            return Some(out);
        }
        cursor = parent;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rooted() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("inside")).expect("inside");
        std::fs::write(dir.path().join("inside/file.txt"), "x").expect("file");
        dir
    }

    #[test]
    fn a_path_inside_the_root_resolves() {
        let dir = rooted();
        let resolved = resolve_in_scope(dir.path(), &dir.path().join("inside/file.txt"))
            .expect("a path under the root is in scope");
        assert!(resolved.ends_with("inside/file.txt"), "{resolved:?}");
    }

    #[test]
    fn a_traversal_out_of_the_root_is_refused() {
        let dir = rooted();
        let error = resolve_in_scope(dir.path(), &dir.path().join("inside/../../elsewhere"))
            .expect_err("../ must not leave the root");
        assert!(
            matches!(error, WorkspaceError::PathEscape { .. }),
            "{error:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_pointing_out_of_the_root_is_refused() {
        // The case a string-prefix check misses entirely: the path looks like
        // it is inside the root right up until the filesystem resolves it.
        let dir = rooted();
        let outside = tempfile::tempdir().expect("outside");
        std::fs::write(outside.path().join("secret.txt"), "s").expect("secret");
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            dir.path().join("inside/link.txt"),
        )
        .expect("symlink");

        let error = resolve_in_scope(dir.path(), &dir.path().join("inside/link.txt"))
            .expect_err("a symlink out of the root is an escape");
        assert!(
            matches!(error, WorkspaceError::PathEscape { .. }),
            "{error:?}"
        );
    }

    #[test]
    fn a_path_that_does_not_exist_yet_is_judged_by_its_parent() {
        // A worktree directory is named before it is created, so the check
        // has to work on a path with no inode.
        let dir = rooted();
        let resolved = resolve_in_scope(dir.path(), &dir.path().join("inside/not-yet"))
            .expect("a new name under an existing parent is in scope");
        assert!(resolved.ends_with("inside/not-yet"), "{resolved:?}");
    }

    #[test]
    fn a_new_path_under_a_parent_outside_the_root_is_refused() {
        let dir = rooted();
        let outside = tempfile::tempdir().expect("outside");
        let error = resolve_in_scope(dir.path(), &outside.path().join("not-yet"))
            .expect_err("a new name outside the root is still outside");
        assert!(
            matches!(error, WorkspaceError::PathEscape { .. }),
            "{error:?}"
        );
    }
}
