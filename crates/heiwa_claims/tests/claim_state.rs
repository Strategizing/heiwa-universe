//! The state ladder and the manifest security boundary.
//!
//! The state tests are exhaustive by design: every branch of `state::compute`
//! is a way the registry could lie, and a registry that lies is worse than no
//! registry because it launders a false claim through a mechanism that looks
//! rigorous.

use std::collections::BTreeMap;

use heiwa_claims::evidence::{Environment, EvidenceRecord, VerifyResult};
use heiwa_claims::manifest::{Claim, Expiry, RequiredState};
use heiwa_claims::state::{compute, ClaimState, Observation};
use heiwa_claims::verifier::{self, VerifierParams};

const DIGEST: &str = "digest-a";

fn claim() -> Claim {
    Claim {
        claim_id: "test.claim".into(),
        subject: "crates/heiwa_claims".into(),
        claim: "the registry computes state instead of accepting it".into(),
        required_state: RequiredState::Verified,
        scope: vec!["crates/heiwa_claims".into()],
        verifier_id: "cargo-test".into(),
        params: VerifierParams {
            package: Some("heiwa_claims".into()),
            ..Default::default()
        },
        compatibility: vec![],
        expiry: Expiry::default(),
        retired: false,
    }
}

fn passing_evidence() -> EvidenceRecord {
    EvidenceRecord {
        claim_id: "test.claim".into(),
        verifier_id: "cargo-test".into(),
        verifier_version: "1".into(),
        result: VerifyResult::Pass,
        commit: "abc123".into(),
        scope_digest: DIGEST.into(),
        verified_at: 1_000_000,
        environment: Environment {
            os: "macos".into(),
            arch: "aarch64".into(),
        },
        detail: String::new(),
    }
}

fn observe<'a>(evidence: Option<&'a EvidenceRecord>, scope_files: usize) -> Observation<'a> {
    Observation {
        scope_files,
        scope_digest: DIGEST,
        evidence,
        evidence_is_ancestor: evidence.is_some(),
        verifier_version: "1",
        now: 1_000_000,
    }
}

#[test]
fn fresh_bound_evidence_verifies() {
    let evidence = passing_evidence();
    let status = compute(&claim(), &observe(Some(&evidence), 9));
    assert_eq!(status.state, ClaimState::Verified, "{}", status.reason);
    assert!(status.state.satisfies(RequiredState::Verified));
}

#[test]
fn subject_present_without_evidence_is_implemented_not_verified() {
    let status = compute(&claim(), &observe(None, 9));
    assert_eq!(status.state, ClaimState::Implemented);
    // The distinction that matters: existing is not proving.
    assert!(!status.state.satisfies(RequiredState::Verified));
    assert!(status.state.satisfies(RequiredState::Implemented));
}

#[test]
fn empty_scope_without_history_is_planned() {
    let status = compute(&claim(), &observe(None, 0));
    assert_eq!(status.state, ClaimState::Planned);
    assert!(!status.state.satisfies(RequiredState::Implemented));
}

#[test]
fn empty_scope_with_history_is_retired_not_planned() {
    // Code that was proven and then deleted is a retirement someone forgot to
    // declare, not a fresh plan. Collapsing the two would let a removal look
    // like a roadmap item.
    let evidence = passing_evidence();
    let status = compute(&claim(), &observe(Some(&evidence), 0));
    assert_eq!(status.state, ClaimState::Retired, "{}", status.reason);
}

#[test]
fn manifest_retirement_wins_over_everything() {
    let mut c = claim();
    c.retired = true;
    let evidence = passing_evidence();
    let status = compute(&c, &observe(Some(&evidence), 9));
    assert_eq!(status.state, ClaimState::Retired);
}

#[test]
fn changed_scope_degrades_standing_evidence() {
    let evidence = passing_evidence();
    let mut obs = observe(Some(&evidence), 9);
    obs.scope_digest = "digest-b";
    let status = compute(&claim(), &obs);
    assert_eq!(status.state, ClaimState::Degraded, "{}", status.reason);
    assert!(!status.state.satisfies(RequiredState::Implemented));
}

#[test]
fn changed_verifier_semantics_degrade_standing_evidence() {
    let evidence = passing_evidence();
    let mut obs = observe(Some(&evidence), 9);
    obs.verifier_version = "2";
    assert_eq!(compute(&claim(), &obs).state, ClaimState::Degraded);
}

#[test]
fn evidence_from_a_diverged_branch_degrades() {
    let evidence = passing_evidence();
    let mut obs = observe(Some(&evidence), 9);
    obs.evidence_is_ancestor = false;
    assert_eq!(compute(&claim(), &obs).state, ClaimState::Degraded);
}

#[test]
fn a_failing_run_degrades_rather_than_reverting_to_implemented() {
    let mut evidence = passing_evidence();
    evidence.result = VerifyResult::Fail;
    let status = compute(&claim(), &observe(Some(&evidence), 9));
    assert_eq!(status.state, ClaimState::Degraded);
    // A proven-false claim must not be indistinguishable from an unproven one.
    assert!(!status.state.satisfies(RequiredState::Implemented));
}

#[test]
fn expiry_degrades_evidence_that_outlived_its_freshness_policy() {
    let mut c = claim();
    c.expiry = Expiry {
        max_age_days: Some(30),
    };
    let evidence = passing_evidence();
    let mut obs = observe(Some(&evidence), 9);
    obs.now = evidence.verified_at + 31 * 86_400;
    assert_eq!(compute(&c, &obs).state, ClaimState::Degraded);

    obs.now = evidence.verified_at + 29 * 86_400;
    assert_eq!(compute(&c, &obs).state, ClaimState::Verified);
}

#[test]
fn absent_expiry_means_scope_is_the_only_staleness_signal() {
    let evidence = passing_evidence();
    let mut obs = observe(Some(&evidence), 9);
    obs.now = evidence.verified_at + 3650 * 86_400;
    assert_eq!(compute(&claim(), &obs).state, ClaimState::Verified);
}

// ── Verifier allowlist ──────────────────────────────────────────────────────

fn packages() -> BTreeMap<String, String> {
    BTreeMap::from([(
        "heiwa_claims".to_string(),
        "crates/heiwa_claims".to_string(),
    )])
}

#[test]
fn every_allowlisted_verifier_has_a_unique_id() {
    let mut ids: Vec<&str> = verifier::VERIFIERS.iter().map(|v| v.id).collect();
    let count = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), count, "duplicate verifier id in the allowlist");
}

#[test]
fn a_manifest_cannot_name_a_verifier_that_is_not_allowlisted() {
    assert!(verifier::lookup("bash -c rm -rf /").is_none());
    assert!(verifier::lookup("scripts/anything.sh").is_none());
}

#[test]
fn cargo_test_rejects_a_package_this_workspace_does_not_declare() {
    let def = verifier::lookup("cargo-test").expect("cargo-test is allowlisted");
    let params = VerifierParams {
        package: Some("; rm -rf /".into()),
        ..Default::default()
    };
    assert!(params.validate(def, &packages()).is_err());

    let real = VerifierParams {
        package: Some("heiwa_claims".into()),
        ..Default::default()
    };
    assert!(real.validate(def, &packages()).is_ok());
}

#[test]
fn a_verifier_rejects_parameters_it_does_not_consume() {
    let def = verifier::lookup("l0-acceptance").expect("l0-acceptance is allowlisted");
    let params = VerifierParams {
        package: Some("heiwa_claims".into()),
        ..Default::default()
    };
    // Scripts take no caller-supplied input at all; silently ignoring a
    // parameter would make the manifest look like it constrained something.
    assert!(params.validate(def, &packages()).is_err());
    assert!(VerifierParams::default().validate(def, &packages()).is_ok());
}

#[test]
fn text_verifiers_require_the_thing_they_check() {
    let symbols = verifier::lookup("symbols-present").unwrap();
    assert!(VerifierParams::default()
        .validate(symbols, &packages())
        .is_err());

    let absent = verifier::lookup("text-absent").unwrap();
    assert!(VerifierParams::default()
        .validate(absent, &packages())
        .is_err());
}
