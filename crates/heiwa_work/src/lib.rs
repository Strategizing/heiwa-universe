//! Durable `Work` — the coordination aggregate above threads and tasks.
//!
//! `Work` is a fold over operator-domain events, not a second store. The
//! runtime appends those events through `OperatorSessionService`, which stays
//! the only local domain writer; this crate is I/O-free and takes events as
//! input. See `docs/superpowers/specs/2026-08-22-heiwa-work-fabric-design.md`.

pub mod events;
pub mod model;

pub use events::{
    work_created_event, work_linked_event, WorkCreatedPayload, WorkLinkOrigin, WorkLinkedPayload,
};
pub use model::{Work, WorkId, WorkStatus, SCHEMA_VERSION};

#[derive(Debug, thiserror::Error)]
pub enum WorkError {
    #[error("work record is schema version {0}, newer than this build understands")]
    UnknownVersion(u32),
    #[error("{0}")]
    Malformed(String),
}

pub type Result<T> = std::result::Result<T, WorkError>;
