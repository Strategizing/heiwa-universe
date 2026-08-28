//! The verifier allowlist.
//!
//! A claim manifest names a verifier by id and supplies bounded parameters. It
//! never supplies a command line. This is the load-bearing security property of
//! the registry: a manifest is data that anyone may propose, and data must not
//! be able to execute. Every runnable definition lives here, in tracked Rust,
//! where changing one is a reviewable code change rather than a config edit.
//!
//! Two verifier kinds run in this process and never spawn anything at all
//! (`SymbolsPresent`, `TextAbsent`). They exist because the two drift classes
//! the continuity design names first — a claimed symbol that no longer exists,
//! and retired vocabulary still present in a canonical boundary — are text
//! questions, not build questions, and answering them without a subprocess
//! keeps the registry cheap enough to run on every gate.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::ClaimError;

/// How a verifier establishes its result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifierKind {
    /// In-process. Every named symbol must appear in at least one scope file.
    SymbolsPresent,
    /// In-process. No named pattern may appear in any scope file.
    TextAbsent,
    /// Subprocess with a fixed program and fixed arguments. The manifest
    /// contributes nothing to the command line.
    Script {
        program: &'static str,
        args: &'static [&'static str],
    },
    /// Subprocess running the workspace test suite for one package. The package
    /// name is the only caller-supplied value, and it is accepted only when it
    /// exactly matches a package this workspace actually declares.
    CargoTest,
}

/// One allowlisted verifier.
///
/// `version` is part of the evidence binding, not decoration. A verifier whose
/// meaning changes must bump it, which immediately degrades every claim still
/// resting on the old semantics instead of letting stale proof look current.
#[derive(Debug, Clone, Copy)]
pub struct VerifierDef {
    pub id: &'static str,
    pub version: &'static str,
    pub kind: VerifierKind,
    pub description: &'static str,
}

/// The complete set of verifiers a manifest may reference.
pub const VERIFIERS: &[VerifierDef] = &[
    VerifierDef {
        id: "symbols-present",
        version: "1",
        kind: VerifierKind::SymbolsPresent,
        description: "Every named symbol appears in at least one file under the claim's scope.",
    },
    VerifierDef {
        id: "text-absent",
        version: "1",
        kind: VerifierKind::TextAbsent,
        description: "No named pattern appears in any file under the claim's scope.",
    },
    VerifierDef {
        id: "cargo-test",
        version: "1",
        kind: VerifierKind::CargoTest,
        description: "`cargo test -p <package>` passes for one declared workspace member.",
    },
    VerifierDef {
        id: "l0-acceptance",
        version: "1",
        kind: VerifierKind::Script {
            program: "bash",
            args: &["scripts/check_l0_acceptance.sh"],
        },
        description: "Roadmap L0 acceptance gate.",
    },
    VerifierDef {
        id: "l1-acceptance",
        version: "1",
        kind: VerifierKind::Script {
            program: "bash",
            args: &["scripts/check_l1_acceptance.sh"],
        },
        description: "Roadmap L1 acceptance gate.",
    },
    VerifierDef {
        id: "l2-acceptance",
        version: "1",
        kind: VerifierKind::Script {
            program: "bash",
            args: &["scripts/check_l2_acceptance.sh"],
        },
        description: "Roadmap L2 acceptance gate.",
    },
    VerifierDef {
        id: "agent-baseline",
        version: "1",
        kind: VerifierKind::Script {
            program: "bash",
            args: &["scripts/check_agent_baseline.sh"],
        },
        description: "Repository agent baseline gate.",
    },
];

pub fn lookup(id: &str) -> Option<&'static VerifierDef> {
    VERIFIERS.iter().find(|v| v.id == id)
}

/// Parameters a manifest may hand a verifier.
///
/// Deliberately a closed shape rather than free-form TOML: a verifier can only
/// receive the argument classes some allowlisted verifier already understands,
/// so a new manifest cannot smuggle in a new capability.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifierParams {
    #[serde(default)]
    pub symbols: Vec<String>,
    #[serde(default)]
    pub patterns: Vec<String>,
    #[serde(default)]
    pub package: Option<String>,
}

impl VerifierParams {
    /// Reject a claim whose parameters do not match what its verifier consumes.
    ///
    /// Checked at load time, not at run time, so a malformed manifest fails the
    /// whole registry rather than silently producing a verifier that tests
    /// nothing and reports a pass.
    pub fn validate(
        &self,
        def: &VerifierDef,
        workspace_packages: &BTreeMap<String, String>,
    ) -> Result<(), ClaimError> {
        let unexpected = |field: &str| {
            Err(ClaimError::Params(format!(
                "verifier `{}` takes no `{field}`",
                def.id
            )))
        };
        match def.kind {
            VerifierKind::SymbolsPresent => {
                if self.symbols.is_empty() {
                    return Err(ClaimError::Params(format!(
                        "verifier `{}` requires a non-empty `symbols`",
                        def.id
                    )));
                }
                if !self.patterns.is_empty() {
                    return unexpected("patterns");
                }
                if self.package.is_some() {
                    return unexpected("package");
                }
            }
            VerifierKind::TextAbsent => {
                if self.patterns.is_empty() {
                    return Err(ClaimError::Params(format!(
                        "verifier `{}` requires a non-empty `patterns`",
                        def.id
                    )));
                }
                if !self.symbols.is_empty() {
                    return unexpected("symbols");
                }
                if self.package.is_some() {
                    return unexpected("package");
                }
            }
            VerifierKind::CargoTest => {
                let package = self.package.as_deref().ok_or_else(|| {
                    ClaimError::Params(format!("verifier `{}` requires a `package`", def.id))
                })?;
                // An exact match against the packages this workspace declares is
                // what makes the value inert: there is no string a manifest can
                // write that is not already a package Cargo would build.
                if !workspace_packages.contains_key(package) {
                    return Err(ClaimError::Params(format!(
                        "`package = \"{package}\"` is not a member of this workspace"
                    )));
                }
                if !self.symbols.is_empty() {
                    return unexpected("symbols");
                }
                if !self.patterns.is_empty() {
                    return unexpected("patterns");
                }
            }
            VerifierKind::Script { .. } => {
                if !self.symbols.is_empty() {
                    return unexpected("symbols");
                }
                if !self.patterns.is_empty() {
                    return unexpected("patterns");
                }
                if self.package.is_some() {
                    return unexpected("package");
                }
            }
        }
        Ok(())
    }
}
