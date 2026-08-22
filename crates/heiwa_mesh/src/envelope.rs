//! The replication frame.
//!
//! `heiwa_evidence` already owns append/replay framing, locking, fsync, and
//! sensitive-material rejection. This wraps a domain event for transport
//! without disturbing any of it: one signed, hash-chained record per node.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::clock::HybridLogicalClock;
use crate::node::NodeId;
use crate::{MeshError, Result};

pub const SCHEMA_VERSION: u32 = 1;

/// What may leave this node at all.
///
/// Same vocabulary and same wire strings as `heiwa_core::drex::PrivacyClass`,
/// deliberately not the same type: this crate sits below the runtime and must
/// not depend on an application binary. `privacy_wire_strings_match_drex`
/// fails if the two ever drift apart.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyClass {
    /// Replicable to the user's enrolled nodes and beyond them.
    Standard,
    /// Never leaves the node that produced it.
    LocalOnly,
    /// Replicable between the user's own nodes, never off them.
    Sovereign,
}

impl PrivacyClass {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::LocalOnly => "local_only",
            Self::Sovereign => "sovereign",
        }
    }

    /// Whether an envelope of this class may be sent to an enrolled peer.
    pub fn may_replicate_to_peer(&self) -> bool {
        !matches!(self, Self::LocalOnly)
    }

    /// Whether it may leave the user's own devices — a relay, a backup, a
    /// third party of any kind.
    pub fn may_leave_user_devices(&self) -> bool {
        matches!(self, Self::Standard)
    }
}

/// A domain event, framed for a peer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeshEnvelope {
    pub schema_version: u32,
    pub event_id: String,
    pub origin_node: NodeId,
    /// Monotonic per node. A gap is a missing envelope, not a reordering.
    pub origin_seq: u64,
    pub hlc: HybridLogicalClock,
    /// Event ids this causally follows. Carried in the frame from the first
    /// version because backfilling causality into a replicated log is not
    /// possible; **no logic reads it yet** — anti-entropy is not built.
    pub causal_parents: Vec<String>,
    pub work_id: Option<String>,
    pub task_id: Option<String>,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub privacy_class: PrivacyClass,
    /// Digest of the previous envelope from this node, extending the receipt
    /// hash-chain across the mesh. `None` only for a node's first envelope.
    pub previous_hash: Option<String>,
    /// Lowercase hex Ed25519 signature over the signing digest.
    pub signature: String,
}

/// Length-prefixed, so no two different field sequences can hash alike.
fn feed(hasher: &mut Sha256, label: &str, bytes: &[u8]) {
    hasher.update((label.len() as u64).to_le_bytes());
    hasher.update(label.as_bytes());
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

/// An absent field and an empty one are different facts.
fn feed_optional(hasher: &mut Sha256, label: &str, value: Option<&str>) {
    match value {
        None => feed(hasher, label, &[0u8]),
        Some(value) => {
            let mut buffer = Vec::with_capacity(value.len() + 1);
            buffer.push(1u8);
            buffer.extend_from_slice(value.as_bytes());
            feed(hasher, label, &buffer);
        }
    }
}

/// Serialize with object keys sorted, so the digest depends on the payload's
/// content and not on how some other crate happened to parse it.
///
/// `serde_json::Map` is a `BTreeMap` by default but an `IndexMap` when any
/// crate in the build enables `preserve_order`. A signature must not be
/// decided by a transitive feature flag.
fn canonical_json(value: &serde_json::Value) -> String {
    let mut out = String::new();
    write_canonical(value, &mut out);
    out
}

fn write_canonical(value: &serde_json::Value, out: &mut String) {
    match value {
        serde_json::Value::Null => out.push_str("null"),
        serde_json::Value::Bool(true) => out.push_str("true"),
        serde_json::Value::Bool(false) => out.push_str("false"),
        serde_json::Value::Number(number) => out.push_str(&number.to_string()),
        serde_json::Value::String(text) => {
            out.push_str(&serde_json::Value::String(text.clone()).to_string())
        }
        serde_json::Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            out.push('{');
            for (index, key) in keys.into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::Value::String(key.clone()).to_string());
                out.push(':');
                write_canonical(&map[key], out);
            }
            out.push('}');
        }
    }
}

/// Everything an envelope needs that is not derived or signed.
#[derive(Clone, Debug)]
pub struct EnvelopeDraft {
    pub event_id: String,
    pub origin_seq: u64,
    pub hlc: HybridLogicalClock,
    pub causal_parents: Vec<String>,
    pub work_id: Option<String>,
    pub task_id: Option<String>,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub privacy_class: PrivacyClass,
    pub previous_hash: Option<String>,
}

impl MeshEnvelope {
    /// Frame and sign a draft as this node.
    pub fn seal(node_id: &NodeId, key: &SigningKey, draft: EnvelopeDraft) -> Result<Self> {
        let derived = NodeId::from_public_key(&key.verifying_key());
        if derived != *node_id {
            return Err(MeshError::WrongOrigin {
                claimed: node_id.to_string(),
                expected: derived.to_string(),
            });
        }
        let mut envelope = Self {
            schema_version: SCHEMA_VERSION,
            event_id: draft.event_id,
            origin_node: node_id.clone(),
            origin_seq: draft.origin_seq,
            hlc: draft.hlc,
            causal_parents: draft.causal_parents,
            work_id: draft.work_id,
            task_id: draft.task_id,
            event_type: draft.event_type,
            payload: draft.payload,
            privacy_class: draft.privacy_class,
            previous_hash: draft.previous_hash,
            signature: String::new(),
        };
        envelope.signature = crate::to_hex(&key.sign(&envelope.signing_digest()).to_bytes());
        Ok(envelope)
    }

    /// Verify this envelope against the claimed origin's public key.
    pub fn open(&self, origin: &NodeId, key: &VerifyingKey) -> Result<()> {
        let derived = NodeId::from_public_key(key);
        if derived != *origin {
            return Err(MeshError::WrongOrigin {
                claimed: origin.to_string(),
                expected: derived.to_string(),
            });
        }
        if self.origin_node != *origin {
            return Err(MeshError::WrongOrigin {
                claimed: self.origin_node.to_string(),
                expected: origin.to_string(),
            });
        }
        let signature =
            Signature::from_bytes(&crate::from_hex::<64>(&self.signature, "signature")?);
        key.verify(&self.signing_digest(), &signature)
            .map_err(|_| MeshError::BadSignature {
                origin_node: self.origin_node.to_string(),
            })
    }

    /// The digest this envelope's own successor chains onto.
    pub fn hash(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(self.signing_digest());
        hasher.update(self.signature.as_bytes());
        format!("sha256:{}", crate::to_hex(&hasher.finalize()))
    }

    /// Canonical bytes covered by the signature: every field except the
    /// signature itself.
    fn signing_digest(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        feed(
            &mut hasher,
            "schema_version",
            &self.schema_version.to_le_bytes(),
        );
        feed(&mut hasher, "event_id", self.event_id.as_bytes());
        feed(
            &mut hasher,
            "origin_node",
            self.origin_node.as_str().as_bytes(),
        );
        feed(&mut hasher, "origin_seq", &self.origin_seq.to_le_bytes());
        feed(&mut hasher, "hlc_wall_ms", &self.hlc.wall_ms.to_le_bytes());
        feed(&mut hasher, "hlc_counter", &self.hlc.counter.to_le_bytes());
        feed(
            &mut hasher,
            "causal_parent_count",
            &(self.causal_parents.len() as u64).to_le_bytes(),
        );
        for parent in &self.causal_parents {
            feed(&mut hasher, "causal_parent", parent.as_bytes());
        }
        feed_optional(&mut hasher, "work_id", self.work_id.as_deref());
        feed_optional(&mut hasher, "task_id", self.task_id.as_deref());
        feed(&mut hasher, "event_type", self.event_type.as_bytes());
        feed(
            &mut hasher,
            "payload",
            canonical_json(&self.payload).as_bytes(),
        );
        feed(
            &mut hasher,
            "privacy_class",
            self.privacy_class.as_str().as_bytes(),
        );
        feed_optional(&mut hasher, "previous_hash", self.previous_hash.as_deref());
        hasher.finalize().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn signed(payload: serde_json::Value) -> MeshEnvelope {
        let signing = ed25519_dalek::SigningKey::from_bytes(&[5u8; 32]);
        let node_id = crate::NodeId::from_public_key(&signing.verifying_key());
        MeshEnvelope::seal(
            &node_id,
            &signing,
            EnvelopeDraft {
                event_id: "event-1".to_string(),
                origin_seq: 1,
                hlc: HybridLogicalClock {
                    wall_ms: 10,
                    counter: 0,
                },
                causal_parents: Vec::new(),
                work_id: None,
                task_id: None,
                event_type: "work.created".to_string(),
                payload,
                privacy_class: PrivacyClass::Sovereign,
                previous_hash: None,
            },
        )
        .expect("seal")
    }

    #[test]
    fn the_digest_does_not_depend_on_json_key_order() {
        let one: serde_json::Value =
            serde_json::from_str(r#"{"alpha":1,"beta":{"x":true,"y":false}}"#).expect("json");
        let other: serde_json::Value =
            serde_json::from_str(r#"{"beta":{"y":false,"x":true},"alpha":1}"#).expect("json");

        assert_eq!(
            signed(one).signature,
            signed(other).signature,
            "the same payload must sign identically however it was parsed"
        );
    }

    #[test]
    fn a_reordered_array_is_a_different_payload() {
        let one = serde_json::json!({ "steps": ["a", "b"] });
        let other = serde_json::json!({ "steps": ["b", "a"] });
        assert_ne!(
            signed(one).signature,
            signed(other).signature,
            "array order is content, not formatting"
        );
    }

    #[test]
    fn privacy_wire_strings_match_drex() {
        assert_eq!(PrivacyClass::Standard.as_str(), "standard");
        assert_eq!(PrivacyClass::LocalOnly.as_str(), "local_only");
        assert_eq!(PrivacyClass::Sovereign.as_str(), "sovereign");
    }

    #[test]
    fn local_only_never_leaves_the_node_that_produced_it() {
        assert!(!PrivacyClass::LocalOnly.may_replicate_to_peer());
        assert!(!PrivacyClass::LocalOnly.may_leave_user_devices());
    }

    #[test]
    fn sovereign_replicates_between_the_users_machines_and_no_further() {
        assert!(PrivacyClass::Sovereign.may_replicate_to_peer());
        assert!(
            !PrivacyClass::Sovereign.may_leave_user_devices(),
            "a user-supplied relay is still off-device"
        );
    }

    #[test]
    fn standard_may_replicate_anywhere_the_user_points_it() {
        assert!(PrivacyClass::Standard.may_replicate_to_peer());
        assert!(PrivacyClass::Standard.may_leave_user_devices());
    }
}
