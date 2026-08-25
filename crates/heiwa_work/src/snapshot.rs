//! Bounded snapshot plus typed deltas.
//!
//! The snapshot is a baseline, not a payload resent after every event. The
//! epoch is what keeps that safe: `projection_revision` is monotonic only
//! *within* one fold and restarts whenever the projector rebuilds, so a client
//! holding revision 3 could otherwise accept a delta based on revision 3 from a
//! different fold entirely. Nothing in the resulting data would show it, so the
//! case is excluded by construction rather than detected afterwards.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Identity of one fold of the read model.
///
/// Minted on every projector build — first start, restart, upgrade, schema
/// change, compaction — and never reused across them.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProjectionEpoch(String);

impl ProjectionEpoch {
    /// Derive an epoch from whatever uniquely identifies this fold, so the
    /// value is reproducible in tests and opaque to clients.
    pub fn from_seed(seed: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(seed.as_bytes());
        let digest = hasher.finalize();
        Self(
            digest
                .iter()
                .take(8)
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        )
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ProjectionEpoch {
    fn default() -> Self {
        Self::from_seed("")
    }
}

/// Rows of one collection, keyed by stable id. Bounded and paginated at the
/// delivery boundary; full logs, diffs, and bodies load through their own
/// authorized endpoints.
pub type CollectionRows = BTreeMap<String, serde_json::Value>;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkSessionSnapshotV1 {
    pub work_id: String,
    /// Durable aggregate revision.
    pub work_revision: u64,
    /// Identity of the fold this baseline came from.
    pub projection_epoch: ProjectionEpoch,
    /// Monotonic within `projection_epoch` only.
    pub projection_revision: u64,
    /// Durable operator-stream replay boundary.
    pub operator_cursor: Option<String>,
    pub source_watermarks: BTreeMap<String, String>,
    pub collections: BTreeMap<String, CollectionRows>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkSessionDeltaV1 {
    pub work_id: String,
    pub projection_epoch: ProjectionEpoch,
    pub base_projection_revision: u64,
    pub projection_revision: u64,
    pub operator_cursor: Option<String>,
    pub upserts: BTreeMap<String, CollectionRows>,
    pub removals: BTreeMap<String, Vec<String>>,
}

/// Why a client must discard its projection and refetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResyncReason {
    /// A frame for another Work cannot mutate this projection.
    WorkChanged,
    /// The projector rebuilt; revisions from the old fold mean nothing now.
    EpochChanged,
    /// A delta was missed, replayed, or arrived out of order.
    RevisionGap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeltaApplyOutcome {
    Applied { projection_revision: u64 },
    ResyncRequired { reason: ResyncReason },
}

/// What a client tracks between frames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientProjection {
    pub work_id: String,
    pub epoch: ProjectionEpoch,
    pub projection_revision: u64,
}

impl ClientProjection {
    pub fn from_snapshot(snapshot: &WorkSessionSnapshotV1) -> Self {
        Self {
            work_id: snapshot.work_id.clone(),
            epoch: snapshot.projection_epoch.clone(),
            projection_revision: snapshot.projection_revision,
        }
    }

    /// Decide a delta. Epoch is checked before revision: a matching revision
    /// across two folds is a coincidence, not agreement.
    pub fn accept(&self, delta: &WorkSessionDeltaV1) -> DeltaApplyOutcome {
        if delta.work_id != self.work_id {
            return DeltaApplyOutcome::ResyncRequired {
                reason: ResyncReason::WorkChanged,
            };
        }
        if delta.projection_epoch != self.epoch {
            return DeltaApplyOutcome::ResyncRequired {
                reason: ResyncReason::EpochChanged,
            };
        }
        if delta.base_projection_revision != self.projection_revision
            || delta.projection_revision <= delta.base_projection_revision
        {
            return DeltaApplyOutcome::ResyncRequired {
                reason: ResyncReason::RevisionGap,
            };
        }
        DeltaApplyOutcome::Applied {
            projection_revision: delta.projection_revision,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn epoch(value: &str) -> ProjectionEpoch {
        ProjectionEpoch::from_seed(value)
    }

    fn client() -> ClientProjection {
        ClientProjection {
            work_id: "work-abc".to_string(),
            epoch: epoch("fold-1"),
            projection_revision: 3,
        }
    }

    fn delta(base: u64, next: u64, epoch: ProjectionEpoch) -> WorkSessionDeltaV1 {
        WorkSessionDeltaV1 {
            work_id: "work-abc".to_string(),
            projection_epoch: epoch,
            base_projection_revision: base,
            projection_revision: next,
            operator_cursor: Some("cursor-9".to_string()),
            upserts: Default::default(),
            removals: Default::default(),
        }
    }

    #[test]
    fn a_delta_on_the_expected_epoch_and_revision_applies() {
        let outcome = client().accept(&delta(3, 4, epoch("fold-1")));
        assert_eq!(
            outcome,
            DeltaApplyOutcome::Applied {
                projection_revision: 4
            }
        );
    }

    #[test]
    fn a_revision_gap_forces_a_resync() {
        let outcome = client().accept(&delta(5, 6, epoch("fold-1")));
        assert_eq!(
            outcome,
            DeltaApplyOutcome::ResyncRequired {
                reason: ResyncReason::RevisionGap
            }
        );
    }

    #[test]
    fn a_delta_must_advance_beyond_its_base_revision() {
        assert_eq!(
            client().accept(&delta(3, 3, epoch("fold-1"))),
            DeltaApplyOutcome::ResyncRequired {
                reason: ResyncReason::RevisionGap
            }
        );
    }

    #[test]
    fn a_delta_from_a_different_fold_forces_a_resync_even_at_a_matching_revision() {
        // The bug this exists to prevent: after a projector rebuild the
        // revision restarts, so base 3 matches by coincidence while the two
        // sides describe different folds.
        let outcome = client().accept(&delta(3, 4, epoch("fold-2")));
        assert_eq!(
            outcome,
            DeltaApplyOutcome::ResyncRequired {
                reason: ResyncReason::EpochChanged
            }
        );
    }

    #[test]
    fn a_delta_for_another_work_never_applies_to_this_projection() {
        let mut foreign = delta(3, 4, epoch("fold-1"));
        foreign.work_id = "work-def".to_string();

        assert!(
            matches!(
                client().accept(&foreign),
                DeltaApplyOutcome::ResyncRequired { .. }
            ),
            "matching epoch and revision cannot cross a Work identity boundary"
        );
    }

    #[test]
    fn a_replayed_delta_is_refused_rather_than_applied_twice() {
        let outcome = client().accept(&delta(2, 3, epoch("fold-1")));
        assert_eq!(
            outcome,
            DeltaApplyOutcome::ResyncRequired {
                reason: ResyncReason::RevisionGap
            }
        );
    }

    #[test]
    fn a_fresh_epoch_differs_from_the_one_before_it() {
        assert_ne!(epoch("fold-1"), epoch("fold-2"));
        assert_eq!(epoch("fold-1"), epoch("fold-1"));
    }

    #[test]
    fn a_snapshot_states_the_epoch_and_revision_a_client_must_track() {
        let snapshot = WorkSessionSnapshotV1 {
            work_id: "work-abc".to_string(),
            work_revision: 7,
            projection_epoch: epoch("fold-1"),
            projection_revision: 3,
            operator_cursor: Some("cursor-9".to_string()),
            source_watermarks: Default::default(),
            collections: Default::default(),
        };
        let resumed = ClientProjection::from_snapshot(&snapshot);
        assert_eq!(
            resumed.accept(&delta(3, 4, epoch("fold-1"))),
            DeltaApplyOutcome::Applied {
                projection_revision: 4
            }
        );
    }
}
