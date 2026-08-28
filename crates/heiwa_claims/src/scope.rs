//! Scope resolution and the staleness digest.
//!
//! This generalizes the `# acceptance-scope:` mechanic already proven by the
//! roadmap gates (`scripts/check_l0_acceptance.sh` and
//! `scripts/hooks/stop_ledger_gate.sh`). The rule it encodes is the useful part:
//! evidence goes stale when *what the verifier reads* changes, not when the
//! clock advances and not when some unrelated file is committed. A gate that
//! fires on every commit is a gate people learn to silence.
//!
//! The digest is taken over tracked blob identities, so it deliberately ignores
//! the working tree. This repository's rule is that a claim describes HEAD;
//! uncommitted work is not yet an assertion about anything.

use std::path::Path;
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::ClaimError;

/// One tracked file inside a claim's scope, and the content it had.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ScopeEntry {
    pub path: String,
    pub blob: String,
}

/// Ask git which tracked files a scope resolves to at HEAD.
pub fn resolve(repo_root: &Path, scope: &[String]) -> Result<Vec<ScopeEntry>, ClaimError> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(repo_root)
        .arg("ls-files")
        .arg("-s")
        .arg("--");
    for path in scope {
        cmd.arg(path);
    }
    let out = cmd
        .output()
        .map_err(|e| ClaimError::Io(format!("git ls-files: {e}")))?;
    if !out.status.success() {
        return Err(ClaimError::Io(format!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }

    let mut entries = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        // `<mode> <blob> <stage>\t<path>`
        let (meta, path) = match line.split_once('\t') {
            Some(parts) => parts,
            None => continue,
        };
        let blob = match meta.split_whitespace().nth(1) {
            Some(blob) => blob,
            None => continue,
        };
        entries.push(ScopeEntry {
            path: path.to_string(),
            blob: blob.to_string(),
        });
    }
    entries.sort();
    entries.dedup();
    Ok(entries)
}

/// Digest a resolved scope.
///
/// Path is folded in alongside content so that moving a file invalidates the
/// claim even when the bytes are identical: a claim about `crates/x` is not
/// evidence about `crates/y`.
pub fn digest(entries: &[ScopeEntry]) -> String {
    let mut hasher = Sha256::new();
    for entry in entries {
        hasher.update(entry.path.as_bytes());
        hasher.update(b"\0");
        hasher.update(entry.blob.as_bytes());
        hasher.update(b"\n");
    }
    format!("{:x}", hasher.finalize())
}

/// Whether `candidate` is `head` or one of its ancestors.
///
/// Evidence recorded on a branch that later diverged is evidence about a tree
/// that is not this one, so ancestry — not mere existence — is the test.
pub fn is_ancestor(repo_root: &Path, candidate: &str, head: &str) -> bool {
    if candidate == head {
        return true;
    }
    Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("merge-base")
        .arg("--is-ancestor")
        .arg(candidate)
        .arg(head)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

pub fn head_commit(repo_root: &Path) -> Result<String, ClaimError> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo_root)
        .arg("rev-parse")
        .arg("HEAD")
        .output()
        .map_err(|e| ClaimError::Io(format!("git rev-parse: {e}")))?;
    if !out.status.success() {
        return Err(ClaimError::Io("git rev-parse HEAD failed".into()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
