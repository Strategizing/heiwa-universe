//! Computed claim state.
//!
//! Deliberately pure: every input is passed in, nothing is read from disk or
//! git here. The whole value of the registry rests on this function being
//! obviously right, and a function that shells out is a function nobody can
//! exhaustively test.

use serde::{Deserialize, Serialize};

use crate::evidence::{EvidenceRecord, VerifyResult};
use crate::manifest::{Claim, RequiredState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClaimState {
    /// The subject does not exist in the tree and never has been verified.
    Planned,
    /// The subject exists, but nothing currently proves the claim about it.
    Implemented,
    /// Fresh passing evidence, bound to this exact scope.
    Verified,
    /// Evidence exists but does not currently support the claim — stale scope,
    /// expired freshness, changed verifier, diverged history, or a failing run.
    Degraded,
    /// The subject is intentionally gone, or was verified once and has since
    /// disappeared from the tree.
    Retired,
}

impl ClaimState {
    pub fn as_str(self) -> &'static str {
        match self {
            ClaimState::Planned => "planned",
            ClaimState::Implemented => "implemented",
            ClaimState::Verified => "verified",
            ClaimState::Degraded => "degraded",
            ClaimState::Retired => "retired",
        }
    }

    /// Whether this computed state is good enough for a consumer that requires
    /// `required`.
    ///
    /// `Degraded` and `Retired` satisfy nothing. That is the point: a claim
    /// whose proof went stale must stop being usable, or staleness costs
    /// nothing and the registry is decoration.
    pub fn satisfies(self, required: RequiredState) -> bool {
        matches!(
            (self, required),
            (ClaimState::Verified, _) | (ClaimState::Implemented, RequiredState::Implemented)
        )
    }
}

/// Everything outside the manifest that the computation needs.
#[derive(Debug, Clone)]
pub struct Observation<'a> {
    /// How many tracked files the claim's scope resolves to at HEAD.
    pub scope_files: usize,
    /// Digest of that resolved scope.
    pub scope_digest: &'a str,
    pub evidence: Option<&'a EvidenceRecord>,
    /// Whether the evidence commit is HEAD or an ancestor of it. Meaningless
    /// when there is no evidence; pass `false`.
    pub evidence_is_ancestor: bool,
    /// Current verifier version from the allowlist.
    pub verifier_version: &'a str,
    /// Unix seconds.
    pub now: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaimStatus {
    pub state: ClaimState,
    /// Why the state is what it is, in one line, for a human reading a gate
    /// failure. A registry that says only "degraded" gets ignored.
    pub reason: String,
}

const DAY_SECONDS: i64 = 86_400;

pub fn compute(claim: &Claim, obs: &Observation<'_>) -> ClaimStatus {
    let status = |state: ClaimState, reason: &str| ClaimStatus {
        state,
        reason: reason.to_string(),
    };

    if claim.retired {
        return status(ClaimState::Retired, "manifest marks the subject retired");
    }

    if obs.scope_files == 0 {
        // The two cases are told apart by history, not by the manifest. Code
        // that was proven and then removed is a retirement someone should have
        // declared; code that never existed is still a plan.
        return if obs.evidence.is_some() {
            status(
                ClaimState::Retired,
                "scope resolves to no tracked files but evidence exists — subject was removed",
            )
        } else {
            status(ClaimState::Planned, "scope resolves to no tracked files")
        };
    }

    let Some(evidence) = obs.evidence else {
        return status(
            ClaimState::Implemented,
            "subject exists in the tree; no verification evidence recorded",
        );
    };

    if evidence.result == VerifyResult::Fail {
        return status(ClaimState::Degraded, "last verifier run failed");
    }
    if evidence.verifier_id != claim.verifier_id {
        return status(
            ClaimState::Degraded,
            "evidence was produced by a different verifier than the manifest names",
        );
    }
    if evidence.verifier_version != obs.verifier_version {
        return status(
            ClaimState::Degraded,
            "verifier semantics changed since this evidence was recorded",
        );
    }
    if !obs.evidence_is_ancestor {
        return status(
            ClaimState::Degraded,
            "evidence commit is not an ancestor of HEAD",
        );
    }
    if evidence.scope_digest != obs.scope_digest {
        return status(
            ClaimState::Degraded,
            "scope changed since verification — reverify",
        );
    }
    if let Some(max_age_days) = claim.expiry.max_age_days {
        let age = obs.now.saturating_sub(evidence.verified_at);
        if age > i64::from(max_age_days) * DAY_SECONDS {
            return status(
                ClaimState::Degraded,
                "evidence is older than the claim's freshness policy",
            );
        }
    }

    status(ClaimState::Verified, "fresh evidence bound to this scope")
}
