//! Executable claim registry.
//!
//! Heiwa must not advertise a capability it cannot prove at the exact current
//! build. This crate is the mechanism: a claim is a manifest plus an
//! allowlisted verifier plus evidence bound to the source state that verifier
//! observed, and its observed state is *computed*, never declared.
//!
//! It generalizes a pattern this repository already proved. The roadmap gates
//! carry a `# acceptance-scope:` header, write a stamp on success, and a Stop
//! hook refuses a completion claim whose stamp no longer covers HEAD. That
//! works, but it is provider-specific (`.claude/*-accept-sha`), hard-coded per
//! layer, and unreadable by anything but bash. The registry keeps the mechanic
//! and drops those three limits.
//!
//! What it does not do yet: run external probes, enforce redaction policy, or
//! carry Effect Receipt references. Those arrive with the receipt taxonomy and
//! are named in the continuity design, not implied here.
//!
//! See `docs/superpowers/specs/2026-08-27-heiwa-work-continuity-triple-design.md`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use thiserror::Error;

pub mod evidence;
pub mod manifest;
pub mod scope;
pub mod state;
pub mod verifier;

pub use evidence::{EvidenceRecord, VerifyResult};
pub use manifest::{Claim, Registry, RequiredState};
pub use state::{ClaimState, ClaimStatus, Observation};
pub use verifier::{VerifierDef, VerifierKind, VerifierParams};

#[derive(Debug, Error)]
pub enum ClaimError {
    #[error("io: {0}")]
    Io(String),
    #[error("manifest: {0}")]
    Manifest(String),
    #[error("unknown verifier `{0}` — verifiers must be declared in heiwa_claims::verifier")]
    UnknownVerifier(String),
    #[error("verifier params: {0}")]
    Params(String),
    #[error("scope has uncommitted changes: {0}")]
    DirtyScope(String),
}

pub type Result<T> = std::result::Result<T, ClaimError>;

/// Locate the repository. `HEIWA_CLAIMS_ROOT` overrides so tests and sandboxes
/// never read or write the operator's real registry.
pub fn repo_root() -> Result<PathBuf> {
    if let Ok(root) = std::env::var("HEIWA_CLAIMS_ROOT") {
        return Ok(PathBuf::from(root));
    }
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| ClaimError::Io(format!("git rev-parse: {e}")))?;
    if !out.status.success() {
        return Err(ClaimError::Io("not inside a git repository".into()));
    }
    Ok(PathBuf::from(
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
    ))
}

/// Package name → member path, for every declared workspace member.
///
/// Read from Cargo.toml rather than accepted from a manifest, so `cargo-test`'s
/// one caller-supplied value can only ever be a package Cargo would build.
pub fn workspace_packages(repo_root: &Path) -> Result<BTreeMap<String, String>> {
    let root_manifest = repo_root.join("Cargo.toml");
    let text = std::fs::read_to_string(&root_manifest)
        .map_err(|e| ClaimError::Io(format!("{}: {e}", root_manifest.display())))?;
    let value: toml::Value = toml::from_str(&text)
        .map_err(|e| ClaimError::Manifest(format!("{}: {e}", root_manifest.display())))?;

    let members = value
        .get("workspace")
        .and_then(|w| w.get("members"))
        .and_then(|m| m.as_array())
        .ok_or_else(|| {
            ClaimError::Manifest("root Cargo.toml declares no workspace members".into())
        })?;

    let mut packages = BTreeMap::new();
    for member in members {
        let Some(rel) = member.as_str() else { continue };
        let member_manifest = repo_root.join(rel).join("Cargo.toml");
        let Ok(member_text) = std::fs::read_to_string(&member_manifest) else {
            continue;
        };
        let Ok(member_value) = toml::from_str::<toml::Value>(&member_text) else {
            continue;
        };
        if let Some(name) = member_value
            .get("package")
            .and_then(|p| p.get("name"))
            .and_then(|n| n.as_str())
        {
            packages.insert(name.to_string(), rel.to_string());
        }
    }
    Ok(packages)
}

/// A claim plus everything computed about it right now.
#[derive(Debug, Clone, Serialize)]
pub struct ClaimReport {
    pub claim_id: String,
    pub subject: String,
    pub claim: String,
    pub verifier_id: String,
    pub required_state: RequiredState,
    pub state: ClaimState,
    pub reason: String,
    pub scope_files: usize,
    pub scope_digest: String,
    pub compatibility: Vec<String>,
    /// Whether this claim currently meets what its consumers require.
    pub satisfied: bool,
}

pub fn now_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Compute the current state of every claim in the registry.
pub fn evaluate(repo_root: &Path, registry: &Registry) -> Result<Vec<ClaimReport>> {
    let head = scope::head_commit(repo_root)?;
    let now = now_seconds();
    let mut reports = Vec::with_capacity(registry.claims.len());

    for claim in &registry.claims {
        let def = claim.verifier()?;
        let entries = scope::resolve(repo_root, &claim.scope)?;
        let digest = scope::digest(&entries);
        let record = evidence::load(repo_root, &claim.claim_id);
        let is_ancestor = record
            .as_ref()
            .is_some_and(|r| scope::is_ancestor(repo_root, &r.commit, &head));

        let status = state::compute(
            claim,
            &Observation {
                scope_files: entries.len(),
                scope_digest: &digest,
                evidence: record.as_ref(),
                evidence_is_ancestor: is_ancestor,
                verifier_version: def.version,
                now,
            },
        );

        reports.push(ClaimReport {
            claim_id: claim.claim_id.clone(),
            subject: claim.subject.clone(),
            claim: claim.claim.clone(),
            verifier_id: claim.verifier_id.clone(),
            required_state: claim.required_state,
            satisfied: status.state.satisfies(claim.required_state),
            state: status.state,
            reason: status.reason,
            scope_files: entries.len(),
            scope_digest: digest,
            compatibility: claim.compatibility.clone(),
        });
    }
    Ok(reports)
}

/// Run a claim's verifier and record the result.
///
/// Refuses on a dirty scope. The acceptance gates this replaces write only at a
/// clean exact HEAD for the same reason: evidence names a commit, so it must be
/// evidence *about* that commit, not about a tree that only the operator can
/// see.
pub fn verify(repo_root: &Path, claim: &Claim) -> Result<EvidenceRecord> {
    let def = claim.verifier()?;
    let entries = scope::resolve(repo_root, &claim.scope)?;

    let dirty = dirty_scope_paths(repo_root, &claim.scope)?;
    if !dirty.is_empty() {
        return Err(ClaimError::DirtyScope(dirty.join(", ")));
    }

    let (result, detail) = run_verifier(repo_root, def, &claim.params, &entries)?;

    let record = EvidenceRecord {
        claim_id: claim.claim_id.clone(),
        verifier_id: def.id.to_string(),
        verifier_version: def.version.to_string(),
        result,
        commit: scope::head_commit(repo_root)?,
        scope_digest: scope::digest(&entries),
        verified_at: now_seconds(),
        environment: evidence::Environment::current(),
        detail: EvidenceRecord::truncate_detail(detail),
    };
    evidence::store(repo_root, &record)?;
    Ok(record)
}

fn dirty_scope_paths(repo_root: &Path, scope: &[String]) -> Result<Vec<String>> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(repo_root)
        .arg("status")
        .arg("--porcelain")
        .arg("--");
    for path in scope {
        cmd.arg(path);
    }
    let out = cmd
        .output()
        .map_err(|e| ClaimError::Io(format!("git status: {e}")))?;
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

fn run_verifier(
    repo_root: &Path,
    def: &VerifierDef,
    params: &VerifierParams,
    entries: &[scope::ScopeEntry],
) -> Result<(VerifyResult, String)> {
    match def.kind {
        VerifierKind::SymbolsPresent => {
            let haystack = read_scope(repo_root, entries);
            let missing: Vec<&str> = params
                .symbols
                .iter()
                .map(String::as_str)
                .filter(|symbol| !haystack.contains(*symbol))
                .collect();
            if missing.is_empty() {
                Ok((
                    VerifyResult::Pass,
                    format!("{} symbol(s) present", params.symbols.len()),
                ))
            } else {
                Ok((
                    VerifyResult::Fail,
                    format!("missing symbol(s): {}", missing.join(", ")),
                ))
            }
        }
        VerifierKind::TextAbsent => {
            let mut found = Vec::new();
            for entry in entries {
                let Ok(text) = std::fs::read_to_string(repo_root.join(&entry.path)) else {
                    continue;
                };
                for pattern in &params.patterns {
                    if text.contains(pattern) {
                        found.push(format!("{}: {pattern}", entry.path));
                    }
                }
            }
            if found.is_empty() {
                Ok((
                    VerifyResult::Pass,
                    format!(
                        "{} pattern(s) absent across {} file(s)",
                        params.patterns.len(),
                        entries.len()
                    ),
                ))
            } else {
                Ok((VerifyResult::Fail, format!("present: {}", found.join("; "))))
            }
        }
        VerifierKind::CargoTest => {
            let package = params
                .package
                .as_deref()
                .ok_or_else(|| ClaimError::Params("cargo-test requires a package".into()))?;
            run_command(repo_root, "cargo", &["test", "-p", package, "--quiet"])
        }
        VerifierKind::Script { program, args } => run_command(repo_root, program, args),
    }
}

/// Concatenate the scope's readable files. Binary and unreadable files are
/// skipped rather than failing the run — a claim about symbols is a claim about
/// the text files in its scope.
fn read_scope(repo_root: &Path, entries: &[scope::ScopeEntry]) -> String {
    let mut buf = String::new();
    for entry in entries {
        if let Ok(text) = std::fs::read_to_string(repo_root.join(&entry.path)) {
            buf.push_str(&text);
            buf.push('\n');
        }
    }
    buf
}

fn run_command(repo_root: &Path, program: &str, args: &[&str]) -> Result<(VerifyResult, String)> {
    let out = Command::new(program)
        .args(args)
        .current_dir(repo_root)
        .output()
        .map_err(|e| ClaimError::Io(format!("{program}: {e}")))?;
    let result = if out.status.success() {
        VerifyResult::Pass
    } else {
        VerifyResult::Fail
    };
    // Tails, not heads: a failing build says why at the end.
    let mut detail = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if detail.is_empty() {
        detail = String::from_utf8_lossy(&out.stdout).trim().to_string();
    }
    Ok((result, detail))
}
