//! Enrolled peers.
//!
//! There is no pairing transport yet (D4 is unresolved and needs a spike), so
//! on every installation today this registry is empty. It exists so that
//! "this machine has no peers" is a *read* rather than a hardcoded literal —
//! the surface that reports sync status stops being a promise and starts
//! being a projection.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::node::NodeId;
use crate::Result;

pub const SCHEMA_VERSION: u32 = 1;

/// A node this installation has enrolled with, and what it was trusted for.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Peer {
    pub node_id: NodeId,
    pub display_name: String,
    /// Lowercase hex of the peer's 32-byte Ed25519 public key.
    pub public_key: String,
    pub enrolled_at: String,
    /// Set when the peer is un-enrolled. A revoked peer is kept, not deleted:
    /// its old envelopes must stay verifiable while being refused as new work.
    pub revoked_at: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerRegistry {
    pub version: u32,
    pub peers: Vec<Peer>,
}

impl PeerRegistry {
    /// Node ids that may currently exchange work with this machine.
    pub fn active_peer_ids(&self) -> Vec<String> {
        self.peers
            .iter()
            .filter(|peer| peer.revoked_at.is_none())
            .map(|peer| peer.node_id.to_string())
            .collect()
    }

    pub fn is_active(&self, node_id: &str) -> bool {
        self.peers
            .iter()
            .any(|peer| peer.node_id.as_str() == node_id && peer.revoked_at.is_none())
    }
}

pub fn registry_path_in(runtime_root: &Path) -> PathBuf {
    runtime_root.join("mesh-peers.json")
}

/// The registry, or an empty one when this installation has never paired.
pub fn load_from(runtime_root: &Path) -> Result<PeerRegistry> {
    let path = registry_path_in(runtime_root);
    if !path.exists() {
        return Ok(PeerRegistry {
            version: SCHEMA_VERSION,
            peers: Vec::new(),
        });
    }
    let raw = std::fs::read_to_string(&path)?;
    let registry: PeerRegistry = serde_json::from_str(&raw)
        .map_err(|error| crate::MeshError::Malformed(error.to_string()))?;
    if registry.version > SCHEMA_VERSION {
        return Err(crate::MeshError::UnknownVersion(registry.version));
    }
    Ok(registry)
}
