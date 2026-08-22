//! What a node can do *right now*.
//!
//! The live half of the capability fabric. An advertisement carries its own
//! expiry and is never trusted stale: a peer that has gone away must stop
//! being a scheduling candidate on its own, without anyone noticing it left.

use serde::{Deserialize, Serialize};

use crate::node::NodeId;

/// A schedulable model on some node.
///
/// A *reference*, not the full DREX candidate: health and cost truth stay in
/// `heiwa_provider` and `heiwa_drex`, which will widen their candidate tuple
/// to include `node_id` rather than have this crate re-declare their types.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelEndpointRef {
    pub node_id: NodeId,
    pub provider: String,
    pub model: String,
    pub account_id: Option<String>,
    /// A warm provider session holding context. Addressable, never copied:
    /// the credential behind it stays on the node that holds it.
    pub session_ref: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PowerState {
    OnMains,
    Battery { percent: u8 },
    Unknown,
}

/// Whole-percent pressure. Integers because an advertisement is compared for
/// equality to decide whether to republish, and floats make that unreliable.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadSnapshot {
    pub cpu_percent: u8,
    pub memory_percent: u8,
    pub gpu_percent: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityAdvertisement {
    pub node_id: NodeId,
    pub published_at_ms: u64,
    pub expires_at_ms: u64,
    pub model_endpoints: Vec<ModelEndpointRef>,
    /// Leasable tool classes present on this node.
    pub tools: Vec<String>,
    /// What this node can host or display.
    pub surfaces: Vec<String>,
    /// Repo checkouts, mounted volumes, connected accounts.
    pub resources: Vec<String>,
    pub load: LoadSnapshot,
    pub power: PowerState,
}

impl CapabilityAdvertisement {
    /// Whether this advertisement may still be believed.
    pub fn is_fresh_at(&self, now_ms: u64) -> bool {
        now_ms < self.expires_at_ms
    }

    /// Whether the content — not the timestamps — differs enough to republish.
    pub fn differs_from(&self, previous: &Self) -> bool {
        self.node_id != previous.node_id
            || self.model_endpoints != previous.model_endpoints
            || self.tools != previous.tools
            || self.surfaces != previous.surfaces
            || self.resources != previous.resources
            || self.load != previous.load
            || self.power != previous.power
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn advertisement() -> CapabilityAdvertisement {
        CapabilityAdvertisement {
            node_id: NodeId::from_public_key(
                &ed25519_dalek::SigningKey::from_bytes(&[3u8; 32]).verifying_key(),
            ),
            published_at_ms: 1_000,
            expires_at_ms: 31_000,
            model_endpoints: Vec::new(),
            tools: vec!["shell".to_string()],
            surfaces: vec!["cockpit".to_string()],
            resources: vec!["repo:heiwa-universe".to_string()],
            load: LoadSnapshot::default(),
            power: PowerState::OnMains,
        }
    }

    #[test]
    fn an_advertisement_is_fresh_until_it_expires() {
        let advertisement = advertisement();
        assert!(advertisement.is_fresh_at(30_999));
    }

    #[test]
    fn an_expired_advertisement_is_never_trusted() {
        let advertisement = advertisement();
        assert!(
            !advertisement.is_fresh_at(31_000),
            "a peer that went away must stop being a candidate on its own"
        );
        assert!(!advertisement.is_fresh_at(90_000));
    }

    #[test]
    fn republishing_is_driven_by_content_not_by_the_clock() {
        let first = advertisement();
        let mut later = advertisement();
        later.published_at_ms = 20_000;
        later.expires_at_ms = 50_000;
        assert!(
            !later.differs_from(&first),
            "a heartbeat with identical capability is not a change"
        );

        later.tools.push("browser".to_string());
        assert!(later.differs_from(&first));
    }

    #[test]
    fn a_change_in_pressure_or_power_is_a_change_worth_publishing() {
        let first = advertisement();
        let mut drained = advertisement();
        drained.power = PowerState::Battery { percent: 9 };
        assert!(drained.differs_from(&first));

        let mut busy = advertisement();
        busy.load = LoadSnapshot {
            cpu_percent: 96,
            memory_percent: 80,
            gpu_percent: None,
        };
        assert!(busy.differs_from(&first));
    }
}
