//! The node record: what this machine is, in the mesh's terms.
//!
//! D3 is resolved here in favour of a **sibling record**. `LocalIdentity`
//! stays what L2 made it — contact-free, no key material, safe to read
//! anywhere — and the keypair lives beside it, bound to the same
//! `installation_id`. Every credential and receipt already attributed to the
//! installation stays attributed.

use std::path::{Path, PathBuf};

use ed25519_dalek::{SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::keystore::KeyStore;
use crate::{MeshError, Result};

/// Schema version of the on-disk record.
pub const SCHEMA_VERSION: u32 = 1;

/// Credential-store service name for node signing keys.
pub const KEY_STORE_SERVICE: &str = "heiwa-mesh";

/// A node's stable name: the fingerprint of its public key.
///
/// Rendered `sha256:<64 hex>` so it is self-describing in a journal line and
/// cannot be confused with the opaque `installation_id` beside it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(String);

impl NodeId {
    pub fn from_public_key(public_key: &VerifyingKey) -> Self {
        Self(fingerprint(public_key.as_bytes()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for NodeId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Platform {
    /// Named explicitly: a derived snake_case rename of `MacOS` is `mac_o_s`,
    /// which matches nothing else in the tree.
    #[serde(rename = "macos")]
    MacOS,
    Windows,
    Linux,
    Ios,
    Android,
}

impl Platform {
    /// What this build is running on, as far as the compiler knows.
    pub fn host() -> Self {
        if cfg!(target_os = "macos") {
            Platform::MacOS
        } else if cfg!(target_os = "windows") {
            Platform::Windows
        } else if cfg!(target_os = "ios") {
            Platform::Ios
        } else if cfg!(target_os = "android") {
            Platform::Android
        } else {
            Platform::Linux
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeClass {
    FullNode,
    MobileNode,
}

/// Whether work assigned to this node survives the user looking away.
///
/// A scheduler input, not a judgement: a mobile node is a first-class peer
/// that creates, approves, and steers work; it is simply never assigned work
/// that must outlive the app going to background.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundReliability {
    /// Keeps executing while unattended — a desktop with a supervised runtime.
    Continuous,
    /// Executes only while the application is foregrounded.
    ForegroundOnly,
}

impl BackgroundReliability {
    pub fn for_class(class: NodeClass) -> Self {
        match class {
            NodeClass::FullNode => BackgroundReliability::Continuous,
            NodeClass::MobileNode => BackgroundReliability::ForegroundOnly,
        }
    }
}

/// An enrolled device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MeshNode {
    pub version: u32,
    pub node_id: NodeId,
    /// The L2 identity this node belongs to. One per installation.
    pub installation_id: String,
    /// Lowercase hex of the 32-byte Ed25519 public key. The private half is
    /// never in this record and never under the configuration root.
    pub public_key: String,
    pub display_name: String,
    pub platform: Platform,
    pub class: NodeClass,
    pub enrolled_at: String,
    pub background_reliability: BackgroundReliability,
}

/// What the caller supplies at enrolment. The clock is a parameter because
/// this crate does not read it, so its behavior is reproducible under test.
#[derive(Clone, Debug)]
pub struct NodeEnrolment<'a> {
    pub installation_id: &'a str,
    pub display_name: &'a str,
    pub platform: Platform,
    pub class: NodeClass,
    pub enrolled_at: &'a str,
}

/// Where the record lives under a given root.
pub fn node_path_in(runtime_root: &Path) -> PathBuf {
    runtime_root.join("mesh-node.json")
}

/// The node record for this installation, or `None` before enrolment.
pub fn load_from(runtime_root: &Path) -> Result<Option<MeshNode>> {
    let path = node_path_in(runtime_root);
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)?;
    // Read the version before the record, so a future schema is refused for
    // being newer rather than for failing to deserialize into today's shape.
    let probe: serde_json::Value =
        serde_json::from_str(&raw).map_err(|error| MeshError::Malformed(error.to_string()))?;
    match probe.get("version").and_then(serde_json::Value::as_u64) {
        Some(version) if version > u64::from(SCHEMA_VERSION) => {
            return Err(MeshError::UnknownVersion(version as u32));
        }
        Some(_) => {}
        None => return Err(MeshError::Malformed("record has no version".to_string())),
    }
    let node: MeshNode =
        serde_json::from_str(&raw).map_err(|error| MeshError::Malformed(error.to_string()))?;
    Ok(Some(node))
}

/// Establish this machine's node identity, or return the existing one.
///
/// Idempotent: the node id is what a peer trusts and what evidence is
/// attributed to, so it is never regenerated for an existing installation.
pub fn establish_in(
    runtime_root: &Path,
    keys: &dyn KeyStore,
    enrolment: &NodeEnrolment<'_>,
    generate: impl FnOnce() -> [u8; 32],
) -> Result<MeshNode> {
    if let Some(existing) = load_from(runtime_root)? {
        return Ok(existing);
    }
    let signing = SigningKey::from_bytes(&generate());
    let public = signing.verifying_key();
    let node = MeshNode {
        version: SCHEMA_VERSION,
        node_id: NodeId::from_public_key(&public),
        installation_id: enrolment.installation_id.to_string(),
        public_key: crate::to_hex(public.as_bytes()),
        display_name: enrolment.display_name.trim().to_string(),
        platform: enrolment.platform,
        class: enrolment.class,
        enrolled_at: enrolment.enrolled_at.to_string(),
        background_reliability: BackgroundReliability::for_class(enrolment.class),
    };
    // The key goes to the credential store first. A record without a key is a
    // node that cannot sign; a key without a record is inert.
    keys.store(node.node_id.as_str(), &crate::to_hex(&signing.to_bytes()))?;
    write_to(runtime_root, &node)?;
    Ok(node)
}

fn write_to(runtime_root: &Path, node: &MeshNode) -> Result<()> {
    std::fs::create_dir_all(runtime_root)?;
    let body = serde_json::to_string_pretty(node)
        .map_err(|error| MeshError::Malformed(error.to_string()))?;
    std::fs::write(node_path_in(runtime_root), body)?;
    Ok(())
}

/// Load the signing key for the established node.
pub fn signing_key_in(runtime_root: &Path, keys: &dyn KeyStore) -> Result<SigningKey> {
    let node = load_from(runtime_root)?.ok_or(MeshError::NotEstablished)?;
    let stored = keys
        .load(node.node_id.as_str())?
        .ok_or_else(|| MeshError::MissingSigningKey {
            node_id: node.node_id.to_string(),
        })?;
    let signing = SigningKey::from_bytes(&crate::from_hex::<32>(&stored, "signing key")?);
    if NodeId::from_public_key(&signing.verifying_key()) != node.node_id {
        return Err(MeshError::Malformed(
            "stored signing key does not belong to this node record".to_string(),
        ));
    }
    Ok(signing)
}

/// The public half, from the record alone — no credential store needed.
pub fn verifying_key(node: &MeshNode) -> Result<VerifyingKey> {
    VerifyingKey::from_bytes(&crate::from_hex::<32>(&node.public_key, "public key")?)
        .map_err(|error| MeshError::Malformed(format!("public key: {error}")))
}

/// A fresh 32-byte seed from the OS CSPRNG.
///
/// Separate from `establish_in` so enrolment stays reproducible under test
/// while the shipped path still gets real entropy.
pub fn random_seed() -> [u8; 32] {
    use rand::RngCore;
    let mut seed = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut seed);
    seed
}

pub(crate) fn fingerprint(public_key_bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(public_key_bytes);
    format!("sha256:{}", crate::to_hex(&hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_wire_strings_match_the_rest_of_the_tree() {
        // `env::consts::OS`, the machine manifest, and the desktop surface all
        // say "macos". A derived snake_case rename says "mac_o_s".
        assert_eq!(
            serde_json::to_value(Platform::MacOS).expect("json"),
            "macos"
        );
        assert_eq!(
            serde_json::to_value(Platform::Windows).expect("json"),
            "windows"
        );
        assert_eq!(
            serde_json::to_value(Platform::Linux).expect("json"),
            "linux"
        );
        assert_eq!(serde_json::to_value(Platform::Ios).expect("json"), "ios");
        assert_eq!(
            serde_json::to_value(Platform::Android).expect("json"),
            "android"
        );
    }

    #[test]
    fn node_classes_and_reliability_read_as_the_surfaces_render_them() {
        assert_eq!(
            serde_json::to_value(NodeClass::FullNode).expect("json"),
            "full_node"
        );
        assert_eq!(
            serde_json::to_value(BackgroundReliability::Continuous).expect("json"),
            "continuous"
        );
        assert_eq!(
            serde_json::to_value(BackgroundReliability::ForegroundOnly).expect("json"),
            "foreground_only"
        );
    }
}
