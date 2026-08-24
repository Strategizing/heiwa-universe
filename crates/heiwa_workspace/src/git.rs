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
}

/// Run git in `dir` and return trimmed stdout.
///
/// Arguments are passed as a list, never through a shell, so a branch name or
/// path containing a space or a semicolon is data rather than syntax.
pub fn git(dir: &Path, args: &[&str]) -> Result<String, GitError> {
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
}
