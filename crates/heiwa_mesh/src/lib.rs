//! Mesh node identity and replication framing (L5.0a).
//!
//! What exists here is deliberately smaller than the mesh design. This crate
//! gives one machine a *node identity* — a keypair, a stable fingerprint, a
//! versioned record — plus the two shapes a peer stream is made of: a live
//! `CapabilityAdvertisement` that expires, and a signed `MeshEnvelope` whose
//! tampering is detectable.
//!
//! There is **no transport, no pairing, and no replication**. A node built by
//! this crate has no peers and says so. Everything here is provable on a
//! single machine, which is the whole reason it can land before the two-node
//! slice (L5.0) that needs a second device.
//!
//! See `docs/superpowers/specs/2026-08-20-heiwa-mesh-runtime-design.md`.
//!
//! Secrets: the signing key is written to the OS credential store through
//! [`keystore::KeyStore`], never to the configuration root. The record on disk
//! holds the public key only.

pub mod advertisement;
pub mod clock;
pub mod envelope;
pub mod keystore;
pub mod node;
pub mod peers;

pub use advertisement::{CapabilityAdvertisement, LoadSnapshot, ModelEndpointRef, PowerState};
pub use clock::HybridLogicalClock;
pub use envelope::{MeshEnvelope, PrivacyClass};
pub use keystore::{KeyStore, KeyStoreError, MemoryKeyStore};
pub use node::{BackgroundReliability, MeshNode, NodeClass, NodeId, Platform, SCHEMA_VERSION};
pub use peers::{Peer, PeerRegistry};

/// Everything this crate can refuse to do.
#[derive(Debug, thiserror::Error)]
pub enum MeshError {
    #[error(
        "mesh node record is schema version {0}, newer than this build understands (supports \
         {SCHEMA_VERSION}); upgrade Heiwa rather than overwriting it"
    )]
    UnknownVersion(u32),
    #[error("no mesh node has been established on this installation")]
    NotEstablished,
    #[error("mesh node record is unreadable: {0}")]
    Malformed(String),
    #[error("signing key store: {0}")]
    KeyStore(#[from] KeyStoreError),
    #[error("signing key for {node_id} is absent from the credential store")]
    MissingSigningKey { node_id: String },
    #[error("signature does not verify for origin node {origin_node}")]
    BadSignature { origin_node: String },
    #[error("envelope claims origin {claimed} but was verified against {expected}")]
    WrongOrigin { claimed: String, expected: String },
    #[error("mesh node file: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, MeshError>;

/// Lowercase hex, matching how every other Heiwa digest is rendered.
pub(crate) fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Decode exactly `N` bytes of lowercase hex, naming what failed.
pub(crate) fn from_hex<const N: usize>(hex: &str, what: &str) -> Result<[u8; N]> {
    if hex.len() != N * 2 {
        return Err(MeshError::Malformed(format!(
            "{what} is {} hex characters, expected {}",
            hex.len(),
            N * 2
        )));
    }
    let mut bytes = [0u8; N];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
            .map_err(|error| MeshError::Malformed(format!("{what}: {error}")))?;
    }
    Ok(bytes)
}
