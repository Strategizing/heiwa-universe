//! Node identity, its credential boundary, and the envelope frame.
//!
//! Single machine only. Nothing here pairs, transports, or replicates.

use std::fs;
use std::path::Path;

use heiwa_mesh::envelope::EnvelopeDraft;
use heiwa_mesh::keystore::KeyStore;
use heiwa_mesh::node::{self, NodeClass, NodeEnrolment, Platform};
use heiwa_mesh::peers::{self, Peer, PeerRegistry};
use heiwa_mesh::{
    BackgroundReliability, HybridLogicalClock, MemoryKeyStore, MeshEnvelope, MeshError, NodeId,
    PrivacyClass,
};

const SEED_A: [u8; 32] = [7u8; 32];
const SEED_B: [u8; 32] = [9u8; 32];

fn root() -> tempfile::TempDir {
    tempfile::tempdir().expect("tempdir")
}

fn enrolment<'a>() -> NodeEnrolment<'a> {
    NodeEnrolment {
        installation_id: "installation-1",
        display_name: "Devon's MacBook",
        platform: Platform::MacOS,
        class: NodeClass::FullNode,
        enrolled_at: "2026-08-22T00:00:00Z",
    }
}

fn establish(dir: &Path, keys: &MemoryKeyStore, seed: [u8; 32]) -> heiwa_mesh::MeshNode {
    node::establish_in(dir, keys, &enrolment(), || seed).expect("establish node")
}

#[test]
fn a_fresh_installation_has_no_mesh_node() {
    let dir = root();
    assert_eq!(node::load_from(dir.path()).expect("load"), None);
}

#[test]
fn establishing_a_node_derives_its_id_from_the_public_key() {
    let dir = root();
    let keys = MemoryKeyStore::new();
    let node = establish(dir.path(), &keys, SEED_A);

    let expected_public = ed25519_dalek::SigningKey::from_bytes(&SEED_A).verifying_key();
    assert_eq!(
        node.public_key,
        expected_public
            .to_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>(),
        "the record carries the public half only"
    );
    assert_eq!(
        node.node_id,
        NodeId::from_public_key(&expected_public),
        "the node id is the fingerprint of that key, not an unrelated uuid"
    );
    assert!(
        node.node_id.as_str().starts_with("sha256:"),
        "a node id must say what it is: {}",
        node.node_id
    );
    assert_eq!(node.node_id.as_str().len(), "sha256:".len() + 64);
    assert_eq!(node.installation_id, "installation-1");
    assert_eq!(
        node.background_reliability,
        BackgroundReliability::Continuous
    );
}

#[test]
fn establishing_twice_returns_the_same_node() {
    let dir = root();
    let keys = MemoryKeyStore::new();
    let first = establish(dir.path(), &keys, SEED_A);
    let second = node::establish_in(dir.path(), &keys, &enrolment(), || {
        panic!("an established node must not be re-minted")
    })
    .expect("second establish");

    assert_eq!(first, second);
}

#[test]
fn the_signing_key_never_lands_under_the_configuration_root() {
    let dir = root();
    let keys = MemoryKeyStore::new();
    let node = establish(dir.path(), &keys, SEED_A);

    let secret_hex: String = SEED_A
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let record = fs::read_to_string(node::node_path_in(dir.path())).expect("read record");
    assert!(
        !record.contains(&secret_hex),
        "the private key must never be written to the configuration root"
    );
    assert!(
        keys.load(node.node_id.as_str())
            .expect("key store")
            .is_some(),
        "the private key belongs in the credential store"
    );
}

#[test]
fn a_record_from_a_newer_schema_is_refused_rather_than_overwritten() {
    let dir = root();
    let keys = MemoryKeyStore::new();
    establish(dir.path(), &keys, SEED_A);

    let path = node::node_path_in(dir.path());
    let mut record: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).expect("read")).expect("json");
    record["version"] = serde_json::json!(node::SCHEMA_VERSION + 1);
    let future = serde_json::to_string_pretty(&record).expect("serialize");
    fs::write(&path, &future).expect("write future record");

    let error = node::load_from(dir.path()).expect_err("a future record must be refused");
    assert!(matches!(error, MeshError::UnknownVersion(_)), "{error}");
    assert_eq!(
        fs::read_to_string(&path).expect("re-read"),
        future,
        "refusing must not destroy the record it could not read"
    );
}

#[test]
fn a_missing_signing_key_is_reported_rather_than_re_minted() {
    let dir = root();
    let keys = MemoryKeyStore::new();
    let node = establish(dir.path(), &keys, SEED_A);
    keys.delete(node.node_id.as_str()).expect("delete key");

    let error = node::signing_key_in(dir.path(), &keys).expect_err("key is gone");
    assert!(
        matches!(error, MeshError::MissingSigningKey { .. }),
        "a node whose key vanished has lost its identity and must say so: {error}"
    );
}

#[test]
fn a_fresh_installation_has_no_enrolled_peers() {
    let dir = root();
    let registry = peers::load_from(dir.path()).expect("load peers");
    assert!(registry.active_peer_ids().is_empty());
}

#[test]
fn a_revoked_peer_is_no_longer_active() {
    let registry = PeerRegistry {
        version: peers::SCHEMA_VERSION,
        peers: vec![
            Peer {
                node_id: NodeId::from_public_key(
                    &ed25519_dalek::SigningKey::from_bytes(&SEED_A).verifying_key(),
                ),
                display_name: "MacBook".to_string(),
                public_key: "aa".to_string(),
                enrolled_at: "2026-08-22T00:00:00Z".to_string(),
                revoked_at: None,
            },
            Peer {
                node_id: NodeId::from_public_key(
                    &ed25519_dalek::SigningKey::from_bytes(&SEED_B).verifying_key(),
                ),
                display_name: "Old laptop".to_string(),
                public_key: "bb".to_string(),
                enrolled_at: "2026-08-22T00:00:00Z".to_string(),
                revoked_at: Some("2026-08-22T01:00:00Z".to_string()),
            },
        ],
    };

    let active = registry.active_peer_ids();
    assert_eq!(active.len(), 1, "a revoked peer must not schedule work");
    assert!(registry.is_active(active[0].as_str()));
    assert!(!registry.is_active(registry.peers[1].node_id.as_str()));
}

fn draft(seq: u64, previous_hash: Option<String>) -> EnvelopeDraft {
    EnvelopeDraft {
        event_id: format!("event-{seq}"),
        origin_seq: seq,
        hlc: HybridLogicalClock {
            wall_ms: 1_700_000_000_000,
            counter: 0,
        },
        causal_parents: Vec::new(),
        work_id: Some("work-1".to_string()),
        task_id: None,
        event_type: "work.created".to_string(),
        payload: serde_json::json!({ "title": "prepare release" }),
        privacy_class: PrivacyClass::Sovereign,
        previous_hash,
    }
}

#[test]
fn a_sealed_envelope_opens_under_its_origin_node() {
    let dir = root();
    let keys = MemoryKeyStore::new();
    let node = establish(dir.path(), &keys, SEED_A);
    let signing = node::signing_key_in(dir.path(), &keys).expect("signing key");

    let envelope = MeshEnvelope::seal(&node.node_id, &signing, draft(1, None)).expect("seal");
    let verifying = node::verifying_key(&node).expect("verifying key");

    envelope
        .open(&node.node_id, &verifying)
        .expect("a sealed envelope opens under the node that sealed it");
    assert_eq!(envelope.origin_node, node.node_id);
    assert_eq!(envelope.work_id.as_deref(), Some("work-1"));
}

#[test]
fn a_tampered_payload_fails_to_open() {
    let dir = root();
    let keys = MemoryKeyStore::new();
    let node = establish(dir.path(), &keys, SEED_A);
    let signing = node::signing_key_in(dir.path(), &keys).expect("signing key");
    let verifying = node::verifying_key(&node).expect("verifying key");

    let mut envelope = MeshEnvelope::seal(&node.node_id, &signing, draft(1, None)).expect("seal");
    envelope.payload = serde_json::json!({ "title": "transfer funds" });

    let error = envelope
        .open(&node.node_id, &verifying)
        .expect_err("a rewritten payload must not verify");
    assert!(matches!(error, MeshError::BadSignature { .. }), "{error}");
}

#[test]
fn a_rewritten_privacy_class_fails_to_open() {
    let dir = root();
    let keys = MemoryKeyStore::new();
    let node = establish(dir.path(), &keys, SEED_A);
    let signing = node::signing_key_in(dir.path(), &keys).expect("signing key");
    let verifying = node::verifying_key(&node).expect("verifying key");

    let mut envelope = MeshEnvelope::seal(&node.node_id, &signing, draft(1, None)).expect("seal");
    envelope.privacy_class = PrivacyClass::Standard;

    let error = envelope
        .open(&node.node_id, &verifying)
        .expect_err("privacy class is covered by the signature");
    assert!(matches!(error, MeshError::BadSignature { .. }), "{error}");
}

#[test]
fn an_envelope_verified_against_another_nodes_key_is_refused() {
    let dir = root();
    let keys = MemoryKeyStore::new();
    let node = establish(dir.path(), &keys, SEED_A);
    let signing = node::signing_key_in(dir.path(), &keys).expect("signing key");

    let envelope = MeshEnvelope::seal(&node.node_id, &signing, draft(1, None)).expect("seal");
    let other = ed25519_dalek::SigningKey::from_bytes(&SEED_B).verifying_key();
    let other_id = NodeId::from_public_key(&other);

    let error = envelope
        .open(&other_id, &other)
        .expect_err("another node's key must not open this envelope");
    assert!(matches!(error, MeshError::WrongOrigin { .. }), "{error}");
}

#[test]
fn each_envelope_chains_onto_its_predecessor() {
    let dir = root();
    let keys = MemoryKeyStore::new();
    let node = establish(dir.path(), &keys, SEED_A);
    let signing = node::signing_key_in(dir.path(), &keys).expect("signing key");

    let first = MeshEnvelope::seal(&node.node_id, &signing, draft(1, None)).expect("seal first");
    let second = MeshEnvelope::seal(&node.node_id, &signing, draft(2, Some(first.hash())))
        .expect("seal second");

    assert_eq!(
        first.previous_hash, None,
        "a node's first envelope has none"
    );
    assert_eq!(second.previous_hash.as_deref(), Some(first.hash().as_str()));
    assert_ne!(
        first.hash(),
        second.hash(),
        "two envelopes must not share a digest"
    );
}
