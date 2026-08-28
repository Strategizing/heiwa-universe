//! Claim manifests.
//!
//! A manifest states what Heiwa asserts and how that assertion can be falsified.
//! It deliberately cannot state whether the assertion currently holds — observed
//! state is computed in `state.rs` from evidence bound to source. A registry
//! where prose can declare itself verified is the failure this crate exists to
//! prevent.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::verifier::{self, VerifierDef, VerifierParams};
use crate::ClaimError;

/// The state a consumer requires before it may rely on a claim.
///
/// Only these two are expressible. `degraded` and `retired` are outcomes, never
/// requirements, and `planned` as a requirement would mean "I need nothing",
/// which is not a claim worth tracking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequiredState {
    Implemented,
    Verified,
}

/// When standing evidence stops counting as current.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expiry {
    /// Wall-clock ceiling on evidence age. Absent means the scope digest is the
    /// only staleness signal — correct for claims about source that cannot rot
    /// on their own, wrong for claims that depend on external behavior.
    #[serde(default)]
    pub max_age_days: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Claim {
    /// Stable repository-wide identifier. Renaming one orphans its evidence,
    /// which is the intended cost: an identifier is part of the claim.
    pub claim_id: String,
    /// The schema, behavior, adapter, surface, or profile being claimed about.
    pub subject: String,
    /// One precise, testable statement.
    pub claim: String,
    pub required_state: RequiredState,
    /// Repository-relative paths whose change can invalidate the claim. This is
    /// both what the verifier reads and what the staleness digest covers, so a
    /// scope that is too narrow produces confident lies and one that is too wide
    /// produces noise nobody will keep answering.
    pub scope: Vec<String>,
    pub verifier_id: String,
    #[serde(default)]
    pub params: VerifierParams,
    /// Schema versions or profiles this claim covers. Reported, not enforced —
    /// enforcement arrives with the conformance corpus.
    #[serde(default)]
    pub compatibility: Vec<String>,
    #[serde(default)]
    pub expiry: Expiry,
    /// Set when the subject is intentionally gone. Distinguishes a deliberate
    /// retirement from a claim that merely lost its files.
    #[serde(default)]
    pub retired: bool,
}

impl Claim {
    pub fn verifier(&self) -> Result<&'static VerifierDef, ClaimError> {
        verifier::lookup(&self.verifier_id)
            .ok_or_else(|| ClaimError::UnknownVerifier(self.verifier_id.clone()))
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestFile {
    #[serde(default, rename = "claim")]
    claims: Vec<Claim>,
}

/// Every claim tracked in this repository.
#[derive(Debug, Clone, Default)]
pub struct Registry {
    pub claims: Vec<Claim>,
}

impl Registry {
    /// Load and validate `claims/*.toml`.
    ///
    /// Validation is total: one bad claim fails the load rather than being
    /// skipped, because a registry that quietly drops what it cannot parse
    /// reports "no failures" for exactly the claims nobody checked.
    pub fn load(repo_root: &Path) -> Result<Self, ClaimError> {
        let dir = repo_root.join("claims");
        let workspace_packages = crate::workspace_packages(repo_root)?;

        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .map_err(|e| ClaimError::Io(format!("{}: {e}", dir.display())))?
            .filter_map(|entry| entry.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|ext| ext == "toml"))
            .collect();
        files.sort();

        let mut claims: Vec<Claim> = Vec::new();
        for path in files {
            let text = std::fs::read_to_string(&path)
                .map_err(|e| ClaimError::Io(format!("{}: {e}", path.display())))?;
            let parsed: ManifestFile = toml::from_str(&text)
                .map_err(|e| ClaimError::Manifest(format!("{}: {e}", path.display())))?;
            claims.extend(parsed.claims);
        }

        let mut seen: BTreeMap<String, ()> = BTreeMap::new();
        for claim in &claims {
            if seen.insert(claim.claim_id.clone(), ()).is_some() {
                return Err(ClaimError::Manifest(format!(
                    "duplicate claim_id `{}`",
                    claim.claim_id
                )));
            }
            if claim.scope.is_empty() {
                return Err(ClaimError::Manifest(format!(
                    "claim `{}` declares no scope",
                    claim.claim_id
                )));
            }
            // A scope that escapes the repository would let a claim rest on
            // evidence from a tree nobody reviews.
            for path in &claim.scope {
                if path.starts_with('/') || path.split('/').any(|seg| seg == "..") {
                    return Err(ClaimError::Manifest(format!(
                        "claim `{}` scope `{path}` leaves the repository",
                        claim.claim_id
                    )));
                }
            }
            let def = claim.verifier()?;
            claim.params.validate(def, &workspace_packages)?;
        }

        claims.sort_by(|a, b| a.claim_id.cmp(&b.claim_id));
        Ok(Self { claims })
    }

    pub fn get(&self, claim_id: &str) -> Option<&Claim> {
        self.claims.iter().find(|c| c.claim_id == claim_id)
    }
}
