//! Verification evidence.
//!
//! One record per claim, written only by a verifier run, binding the result to
//! the exact source state it observed. Replaces the provider-specific
//! `.claude/l*-accept-sha` stamps with something provider-neutral and
//! machine-readable: a Codex, Gemini, or CI run produces the same record a
//! Claude run does, and a product surface can read it without knowing which
//! agent was at the keyboard.
//!
//! Records are hand-editable in the sense that any tracked file is. That is not
//! a hole — editing one to fake a pass requires also forging the scope digest of
//! a tree you do not control, and the digest is recomputed on every read.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ClaimError;

/// Verifier output is kept for diagnosis, not for archive. Bounded because a
/// claim record is committed: an unbounded tail would put build logs, and
/// eventually whatever a build log happens to print, into git history.
const MAX_DETAIL: usize = 2000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VerifyResult {
    Pass,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Environment {
    pub os: String,
    pub arch: String,
}

impl Environment {
    pub fn current() -> Self {
        Self {
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub claim_id: String,
    pub verifier_id: String,
    /// Bound so that changing a verifier's meaning degrades every claim resting
    /// on the old meaning, instead of leaving stale proof looking current.
    pub verifier_version: String,
    pub result: VerifyResult,
    /// Commit the verifier observed. Ancestry against HEAD is checked on read.
    pub commit: String,
    /// Digest of the claim's scope at `commit`.
    pub scope_digest: String,
    /// Unix seconds. Only consulted when the claim declares an expiry.
    pub verified_at: i64,
    pub environment: Environment,
    #[serde(default)]
    pub detail: String,
}

impl EvidenceRecord {
    pub fn truncate_detail(mut detail: String) -> String {
        if detail.len() > MAX_DETAIL {
            // Cut on a char boundary; verifier output is not guaranteed ASCII.
            let mut end = MAX_DETAIL;
            while end > 0 && !detail.is_char_boundary(end) {
                end -= 1;
            }
            detail.truncate(end);
            detail.push_str("\n… truncated");
        }
        detail
    }
}

pub fn path_for(repo_root: &Path, claim_id: &str) -> PathBuf {
    repo_root
        .join("claims")
        .join("evidence")
        .join(format!("{claim_id}.json"))
}

/// Read standing evidence, if any.
///
/// A record that will not parse is treated as absent rather than as an error:
/// the registry's job is to report an unproven claim, and refusing to run
/// because one file is corrupt would hide every other claim's state too.
pub fn load(repo_root: &Path, claim_id: &str) -> Option<EvidenceRecord> {
    let text = std::fs::read_to_string(path_for(repo_root, claim_id)).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn store(repo_root: &Path, record: &EvidenceRecord) -> Result<PathBuf, ClaimError> {
    let path = path_for(repo_root, &record.claim_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| ClaimError::Io(format!("{}: {e}", parent.display())))?;
    }
    let mut text = serde_json::to_string_pretty(record)
        .map_err(|e| ClaimError::Io(format!("serialize evidence: {e}")))?;
    text.push('\n');
    std::fs::write(&path, text).map_err(|e| ClaimError::Io(format!("{}: {e}", path.display())))?;
    Ok(path)
}
