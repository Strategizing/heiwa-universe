//! Manifest loading, against a synthetic repository.
//!
//! Loading is where a hostile or careless manifest gets refused. These tests
//! exist because every one of them is a way a claim could end up in the
//! registry without a real verifier behind it.

use std::fs;
use std::path::Path;

use heiwa_claims::evidence::{self, Environment, EvidenceRecord, VerifyResult};
use heiwa_claims::manifest::Registry;

fn scaffold(claims_toml: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    fs::write(
        root.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/demo\"]\nresolver = \"2\"\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("crates/demo")).unwrap();
    fs::write(
        root.join("crates/demo/Cargo.toml"),
        "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    fs::create_dir_all(root.join("claims")).unwrap();
    fs::write(root.join("claims/test.toml"), claims_toml).unwrap();
    dir
}

fn valid_claim(extra: &str) -> String {
    format!(
        r#"
[[claim]]
claim_id = "demo.one"
subject = "crates/demo"
claim = "demo builds"
required_state = "verified"
scope = ["crates/demo"]
verifier_id = "cargo-test"
params = {{ package = "demo" }}
{extra}
"#
    )
}

#[test]
fn a_well_formed_manifest_loads() {
    let dir = scaffold(&valid_claim(""));
    let registry = Registry::load(dir.path()).expect("valid manifest loads");
    assert_eq!(registry.claims.len(), 1);
    assert_eq!(registry.claims[0].claim_id, "demo.one");
}

#[test]
fn an_unknown_verifier_fails_the_whole_load() {
    // Not "skipped with a warning": a registry that drops what it cannot parse
    // reports no failures for exactly the claims nobody checked.
    let dir = scaffold(
        r#"
[[claim]]
claim_id = "demo.one"
subject = "crates/demo"
claim = "demo builds"
required_state = "verified"
scope = ["crates/demo"]
verifier_id = "make-it-so"
"#,
    );
    let err = Registry::load(dir.path()).unwrap_err().to_string();
    assert!(err.contains("make-it-so"), "{err}");
}

#[test]
fn a_scope_that_leaves_the_repository_is_refused() {
    for bad in ["../secrets", "/etc/passwd", "crates/../../elsewhere"] {
        let dir = scaffold(&format!(
            r#"
[[claim]]
claim_id = "demo.one"
subject = "x"
claim = "x"
required_state = "verified"
scope = ["{bad}"]
verifier_id = "cargo-test"
params = {{ package = "demo" }}
"#
        ));
        assert!(
            Registry::load(dir.path()).is_err(),
            "scope `{bad}` should be refused"
        );
    }
}

#[test]
fn an_empty_scope_is_refused() {
    let dir = scaffold(
        r#"
[[claim]]
claim_id = "demo.one"
subject = "x"
claim = "x"
required_state = "verified"
scope = []
verifier_id = "cargo-test"
params = { package = "demo" }
"#,
    );
    assert!(Registry::load(dir.path()).is_err());
}

#[test]
fn duplicate_claim_ids_are_refused() {
    let mut toml = valid_claim("");
    toml.push_str(&valid_claim(""));
    let dir = scaffold(&toml);
    let err = Registry::load(dir.path()).unwrap_err().to_string();
    assert!(err.contains("duplicate"), "{err}");
}

#[test]
fn a_package_outside_the_workspace_is_refused() {
    let dir = scaffold(
        r#"
[[claim]]
claim_id = "demo.one"
subject = "crates/demo"
claim = "demo builds"
required_state = "verified"
scope = ["crates/demo"]
verifier_id = "cargo-test"
params = { package = "not_a_member" }
"#,
    );
    assert!(Registry::load(dir.path()).is_err());
}

#[test]
fn a_manifest_cannot_declare_its_own_state() {
    // `deny_unknown_fields` is the enforcement. If someone adds `state = ...`
    // to a manifest, the load fails rather than the field being ignored — an
    // ignored field would read, to its author, like it worked.
    let dir = scaffold(&valid_claim(r#"state = "verified""#));
    assert!(Registry::load(dir.path()).is_err());
}

#[test]
fn evidence_round_trips_and_unreadable_evidence_reads_as_absent() {
    let dir = scaffold(&valid_claim(""));
    let root: &Path = dir.path();

    let record = EvidenceRecord {
        claim_id: "demo.one".into(),
        verifier_id: "cargo-test".into(),
        verifier_version: "1".into(),
        result: VerifyResult::Pass,
        commit: "deadbeef".into(),
        scope_digest: "digest".into(),
        verified_at: 42,
        environment: Environment::current(),
        detail: "ok".into(),
    };
    evidence::store(root, &record).expect("store");
    assert_eq!(evidence::load(root, "demo.one").as_ref(), Some(&record));

    fs::write(evidence::path_for(root, "demo.one"), "{ not json").unwrap();
    assert!(
        evidence::load(root, "demo.one").is_none(),
        "corrupt evidence must read as absent so the claim reports unproven, \
         not so the registry refuses to run"
    );
}

#[test]
fn oversized_verifier_output_is_truncated_before_it_reaches_git() {
    let detail = EvidenceRecord::truncate_detail("x".repeat(10_000));
    assert!(detail.len() < 2_200, "len was {}", detail.len());
    assert!(detail.ends_with("truncated"));

    let short = EvidenceRecord::truncate_detail("fine".into());
    assert_eq!(short, "fine");
}

#[test]
fn a_scope_covering_the_evidence_store_is_refused() {
    // Recording proof would invalidate the proof. Such a claim can never reach
    // verified, and a permanently degraded claim with no true cause is how a
    // gate earns the reputation that gets it muted.
    for bad in [
        "claims",
        "claims/",
        "claims/evidence",
        "claims/evidence/x.json",
    ] {
        let dir = scaffold(&format!(
            r#"
[[claim]]
claim_id = "demo.one"
subject = "x"
claim = "x"
required_state = "verified"
scope = ["{bad}"]
verifier_id = "cargo-test"
params = {{ package = "demo" }}
"#
        ));
        let err = Registry::load(dir.path())
            .expect_err(&format!("scope `{bad}` should be refused"))
            .to_string();
        assert!(err.contains("converge"), "{err}");
    }
}

#[test]
fn a_scope_naming_a_claim_manifest_is_still_allowed() {
    // Manifests are fair game: editing what a claim says should invalidate its
    // proof. Only the evidence store is off limits.
    let dir = scaffold(
        r#"
[[claim]]
claim_id = "demo.one"
subject = "x"
claim = "x"
required_state = "verified"
scope = ["claims/test.toml"]
verifier_id = "cargo-test"
params = { package = "demo" }
"#,
    );
    assert!(Registry::load(dir.path()).is_ok());
}
