//! Durable `Work` — the coordination aggregate above threads and tasks.
//!
//! `Work` is a fold over operator-domain events, not a second store. The
//! runtime appends those events through `OperatorSessionService`, which stays
//! the only local domain writer; this crate is I/O-free and takes events as
//! input. See `docs/superpowers/specs/2026-08-22-heiwa-work-fabric-design.md`.

pub mod events;
pub mod migration;
pub mod model;
pub mod projector;
pub mod snapshot;

pub use events::{
    work_created_event, work_linked_event, WorkCreatedPayload, WorkLinkOrigin, WorkLinkedPayload,
};
pub use migration::{resolve_work_id, MigrationConflict, WorkIdResolution};
pub use model::{Work, WorkId, WorkStatus, SCHEMA_VERSION};
pub use projector::{fold, WorkProjection};
pub use snapshot::{
    ClientProjection, DeltaApplyOutcome, ProjectionEpoch, ResyncReason, WorkSessionDeltaV1,
    WorkSessionSnapshotV1,
};

#[derive(Debug, thiserror::Error)]
pub enum WorkError {
    #[error("work record is schema version {0}, newer than this build understands")]
    UnknownVersion(u32),
    #[error("{0}")]
    Malformed(String),
}

pub type Result<T> = std::result::Result<T, WorkError>;
