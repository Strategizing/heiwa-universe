//! What changed, bounded.
//!
//! A projection is delivered to a client, so it is capped by construction. The
//! cap is reported alongside the true total: a shortened diff that does not
//! say it was shortened reads as the whole change, and someone approves it.
//!
//! Full file contents are not carried here. They load through their own
//! request, exactly as the A1-a snapshot contract requires.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::git::git;
use crate::WorkspaceError;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFile {
    pub path: String,
    pub added: u64,
    pub removed: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffProjectionV1 {
    /// At most `limit` entries.
    pub files: Vec<ChangedFile>,
    /// How many files actually changed, before the cap.
    pub total_files: usize,
    pub truncated: bool,
}

/// Project the changes in `worktree`, including files git does not track yet.
pub fn diff_projection_in(
    worktree: &Path,
    limit: usize,
) -> Result<DiffProjectionV1, WorkspaceError> {
    // Staged-and-unstaged tracked changes, with counts.
    let numstat = git(worktree, &["diff", "--numstat", "HEAD"])?;
    let mut files: Vec<ChangedFile> = numstat.lines().filter_map(parse_numstat_line).collect();

    // Untracked files are changes too. `git diff` omits them, and a reviewer
    // who cannot see a newly added file is reviewing the wrong change.
    let untracked = git(worktree, &["ls-files", "--others", "--exclude-standard"])?;
    for path in untracked.lines().filter(|line| !line.trim().is_empty()) {
        let added = std::fs::read_to_string(worktree.join(path))
            .map(|body| body.lines().count() as u64)
            .unwrap_or(0);
        files.push(ChangedFile {
            path: path.to_string(),
            added,
            removed: 0,
        });
    }

    files.sort_by(|left, right| left.path.cmp(&right.path));

    let total_files = files.len();
    let truncated = total_files > limit;
    files.truncate(limit);

    Ok(DiffProjectionV1 {
        files,
        total_files,
        truncated,
    })
}

/// `git diff --numstat` lines are `<added>\t<removed>\t<path>`. A binary file
/// reports `-` for both counts, which becomes zero rather than an error.
fn parse_numstat_line(line: &str) -> Option<ChangedFile> {
    let mut parts = line.splitn(3, '\t');
    let added = parts.next()?;
    let removed = parts.next()?;
    let path = parts.next()?.trim();
    if path.is_empty() {
        return None;
    }
    Some(ChangedFile {
        path: path.to_string(),
        added: added.parse().unwrap_or(0),
        removed: removed.parse().unwrap_or(0),
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestProjectionV1 {
    pub command: String,
    pub passed: bool,
    /// `None` when the process was killed by a signal.
    pub exit_code: Option<i32>,
    /// The tail of combined stdout and stderr, capped.
    pub output: String,
    pub truncated: bool,
}

/// Run a verification command inside `worktree` and record the outcome.
///
/// This is the crate's second and last process boundary, kept separate from
/// [`git`] on purpose: this one runs whatever the user configured as their
/// verification, so it must never inherit git's allowances or its refusals.
///
/// A non-zero exit is a **result**, not an error. A missing binary is an
/// error, because no test actually ran and reporting "failed" would be a lie.
pub fn test_projection_in(
    worktree: &Path,
    command: &str,
    args: &[&str],
    output_limit: usize,
) -> Result<TestProjectionV1, WorkspaceError> {
    let output = std::process::Command::new(command)
        .args(args)
        .current_dir(worktree)
        .output()
        .map_err(|error| WorkspaceError::CommandUnavailable {
            command: command.to_string(),
            reason: error.to_string(),
        })?;

    let mut combined = String::from_utf8_lossy(&output.stdout).to_string();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    let combined = combined.trim().to_string();

    let truncated = combined.len() > output_limit;
    let output_text = if truncated {
        // Keep the tail: a run's failure is at the end, not the beginning.
        // Slice on a char boundary so multi-byte output cannot panic here.
        let start = combined.len() - output_limit;
        let start = (start..combined.len())
            .find(|index| combined.is_char_boundary(*index))
            .unwrap_or(combined.len());
        combined[start..].to_string()
    } else {
        combined
    };

    Ok(TestProjectionV1 {
        command: std::iter::once(command)
            .chain(args.iter().copied())
            .collect::<Vec<_>>()
            .join(" "),
        passed: output.status.success(),
        exit_code: output.status.code(),
        output: output_text,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::repo;
    use crate::worktree::create_worktree_in;

    fn worktree_with_changes(files: usize) -> (tempfile::TempDir, tempfile::TempDir, String) {
        let source = repo();
        let holding = tempfile::tempdir().expect("holding");
        let handle =
            create_worktree_in(source.path(), holding.path(), "work-abc").expect("worktree");
        for index in 0..files {
            std::fs::write(
                std::path::Path::new(&handle.path).join(format!("f{index}.txt")),
                "content\n",
            )
            .expect("write");
        }
        let path = handle.path.clone();
        (source, holding, path)
    }

    #[test]
    fn an_unchanged_worktree_projects_an_empty_diff() {
        let (_source, _holding, path) = worktree_with_changes(0);
        let diff = diff_projection_in(std::path::Path::new(&path), 100).expect("diff");

        assert!(diff.files.is_empty());
        assert_eq!(diff.total_files, 0);
        assert!(!diff.truncated);
    }

    #[test]
    fn a_changed_file_appears_with_its_line_counts() {
        let (_source, _holding, path) = worktree_with_changes(0);
        std::fs::write(std::path::Path::new(&path).join("a.txt"), "one\ntwo\n").expect("edit");

        let diff = diff_projection_in(std::path::Path::new(&path), 100).expect("diff");
        assert_eq!(diff.total_files, 1);
        let file = &diff.files[0];
        assert_eq!(file.path, "a.txt");
        assert_eq!(file.added, 1, "one line added");
        assert_eq!(file.removed, 0);
    }

    #[test]
    fn a_new_file_is_included_even_though_git_does_not_track_it_yet() {
        // An untracked file is a real change the user must see before
        // approving anything. A plain `git diff` would hide it entirely.
        let (_source, _holding, path) = worktree_with_changes(1);
        let diff = diff_projection_in(std::path::Path::new(&path), 100).expect("diff");

        assert_eq!(diff.total_files, 1);
        assert_eq!(diff.files[0].path, "f0.txt");
    }

    #[test]
    fn a_diff_larger_than_the_limit_is_truncated_and_says_so() {
        let (_source, _holding, path) = worktree_with_changes(5);
        let diff = diff_projection_in(std::path::Path::new(&path), 2).expect("diff");

        assert_eq!(diff.files.len(), 2, "only the limit is carried");
        assert_eq!(diff.total_files, 5, "the true count is still reported");
        assert!(
            diff.truncated,
            "a silently shortened diff would be read as the whole change"
        );
    }

    #[test]
    fn a_passing_command_projects_a_pass_with_its_output() {
        let (_source, _holding, path) = worktree_with_changes(0);
        let result = test_projection_in(std::path::Path::new(&path), "echo", &["all good"], 4096)
            .expect("run");

        assert!(result.passed);
        assert_eq!(result.exit_code, Some(0));
        assert!(result.output.contains("all good"), "{}", result.output);
        assert!(!result.truncated);
    }

    #[test]
    fn a_failing_command_projects_a_failure_rather_than_an_error() {
        // A failing test suite is a fact about the code, not a fault in
        // Heiwa. It has to survive into the projection so review can see it.
        let (_source, _holding, path) = worktree_with_changes(0);
        let result =
            test_projection_in(std::path::Path::new(&path), "false", &[], 4096).expect("run");

        assert!(!result.passed);
        assert_ne!(result.exit_code, Some(0));
    }

    #[test]
    fn long_output_keeps_the_tail_and_says_it_was_cut() {
        // The failure is at the end of a test run, so the tail is the part
        // worth keeping.
        let (_source, _holding, path) = worktree_with_changes(0);
        let long = "x".repeat(500);
        let result =
            test_projection_in(std::path::Path::new(&path), "echo", &[&long], 100).expect("run");

        assert!(result.truncated);
        assert!(result.output.len() <= 100, "len {}", result.output.len());
        assert!(
            result.output.ends_with('x'),
            "the tail must be what survives"
        );
    }

    #[test]
    fn a_command_that_does_not_exist_is_an_error_not_a_failed_test() {
        let (_source, _holding, path) = worktree_with_changes(0);
        let error = test_projection_in(
            std::path::Path::new(&path),
            "heiwa-no-such-binary",
            &[],
            4096,
        )
        .expect_err("a missing runner is not a test result");
        assert!(
            matches!(error, WorkspaceError::CommandUnavailable { .. }),
            "{error:?}"
        );
    }
}
