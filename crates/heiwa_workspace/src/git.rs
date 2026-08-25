//! The one place this crate starts a process.
//!
//! Concentrated deliberately: every repository fact in this crate is a fold
//! over the output of this function, so there is exactly one thing to audit
//! for argument injection, one place that decides what a failure means, and
//! one shape for callers to handle.

use std::path::Path;
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("could not run git in {dir}: {source}")]
    Unavailable {
        dir: String,
        #[source]
        source: std::io::Error,
    },
    #[error("git {args} failed in {dir}: {stderr}")]
    Failed {
        dir: String,
        args: String,
        stderr: String,
    },
    #[error("refused to run `git {args}` in {dir}: it can discard uncommitted work")]
    Refused { dir: String, args: String },
}

/// Commands that can destroy work the user has not committed.
///
/// Heiwa mutates inside its own worktrees, which are built from commits, so it
/// never needs any of these. Refusing them here — at the single process
/// boundary — means no future call site can reach them by accident, and the
/// list is one place to review rather than a convention to remember.
fn discards_uncommitted_work(args: &[&str]) -> bool {
    match args {
        ["reset", rest @ ..] => rest.contains(&"--hard"),
        ["checkout", rest @ ..] => rest.contains(&"-f") || rest.contains(&"--force"),
        ["stash", ..] => true,
        ["clean", ..] => true,
        ["restore", ..] => true,
        _ => false,
    }
}

/// Run git in `dir` and return trimmed stdout.
///
/// Arguments are passed as a list, never through a shell, so a branch name or
/// path containing a space or a semicolon is data rather than syntax.
pub fn git(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    if discards_uncommitted_work(args) {
        return Err(GitError::Refused {
            dir: dir.display().to_string(),
            args: args.join(" "),
        });
    }

    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|source| GitError::Unavailable {
            dir: dir.display().to_string(),
            source,
        })?;

    if !output.status.success() {
        return Err(GitError::Failed {
            dir: dir.display().to_string(),
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A real repository with one commit. Real git, because a fake would have
    /// to model git's output formats and a fake that lies is worse than no
    /// test at all.
    pub(crate) fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path();
        git(path, &["init", "-q", "-b", "main", "."]).expect("init");
        git(path, &["config", "user.email", "test@heiwa.ltd"]).expect("email");
        git(path, &["config", "user.name", "test"]).expect("name");
        std::fs::write(path.join("a.txt"), "one\n").expect("write");
        git(path, &["add", "."]).expect("add");
        git(path, &["commit", "-qm", "one"]).expect("commit");
        dir
    }

    #[test]
    fn a_successful_command_returns_trimmed_stdout() {
        let dir = repo();
        let head = git(dir.path(), &["rev-parse", "HEAD"]).expect("rev-parse");
        assert_eq!(head.len(), 40, "a sha and nothing else: {head:?}");
        assert!(!head.ends_with('\n'), "output must be trimmed");
    }

    #[test]
    fn a_failing_command_reports_the_stderr_git_gave() {
        let dir = repo();
        let error =
            git(dir.path(), &["rev-parse", "no-such-ref"]).expect_err("an unknown ref must fail");

        let GitError::Failed { args, stderr, .. } = &error else {
            panic!("expected a Failed variant, got {error:?}");
        };
        assert!(args.contains("no-such-ref"), "{args}");
        assert!(
            !stderr.is_empty(),
            "swallowing git's own message leaves the caller guessing"
        );
    }

    #[test]
    fn a_directory_that_is_not_a_repository_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = git(dir.path(), &["rev-parse", "HEAD"]).expect_err("not a repo");
        assert!(matches!(error, GitError::Failed { .. }));
    }

    #[test]
    fn commands_that_discard_uncommitted_work_are_refused() {
        let dir = repo();
        std::fs::write(dir.path().join("a.txt"), "one\nMINE\n").expect("dirty it");

        for destructive in [
            vec!["reset", "--hard"],
            vec!["checkout", "-f", "main"],
            vec!["stash"],
            vec!["stash", "push"],
            vec!["clean", "-fd"],
            vec!["restore", "a.txt"],
        ] {
            let error = git(dir.path(), &destructive)
                .expect_err("this command can destroy the user's work");
            assert!(
                matches!(error, GitError::Refused { .. }),
                "{destructive:?} must be refused, got {error:?}"
            );
        }

        let survived = std::fs::read_to_string(dir.path().join("a.txt")).expect("read");
        assert_eq!(survived, "one\nMINE\n", "nothing may have run");
    }

    #[test]
    fn a_refusal_names_the_command_it_refused() {
        let dir = repo();
        let error = git(dir.path(), &["reset", "--hard"]).expect_err("refused");
        let GitError::Refused { args, .. } = &error else {
            panic!("expected Refused, got {error:?}");
        };
        assert!(args.contains("reset"), "{args}");
    }

    #[test]
    fn reading_and_worktree_commands_are_still_allowed() {
        let dir = repo();
        git(dir.path(), &["status", "--porcelain=v1"]).expect("status is a read");
        git(dir.path(), &["rev-parse", "HEAD"]).expect("rev-parse is a read");
        git(dir.path(), &["worktree", "list"]).expect("worktree list is a read");
    }
}
