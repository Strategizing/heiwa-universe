//! Provider-owned worker and durable terminal pane identity, bound to `Work`.
//!
//! I/O-free by construction, like `heiwa_work`: the shell spawns processes and
//! appends events, this crate says what a worker and a pane *are* and folds the
//! operator stream into the `runs` read model. See
//! `docs/superpowers/specs/2026-08-22-heiwa-work-fabric-design.md`.

pub mod events;
pub mod model;
pub mod pane;
pub mod projector;

pub use events::{
    pane_closed_event, pane_opened_event, worker_exited_event, worker_heartbeat_event,
    worker_launched_event, PaneClosedPayload, PaneOpenedPayload, WorkerExitedPayload,
    WorkerHeartbeatPayload, WorkerLaunchedPayload,
};
pub use model::{PaneIdentity, PaneState, WorkerIdentity, WorkerState, SCHEMA_VERSION};
pub use pane::{PaneTail, PANE_LINE_BYTES, PANE_TAIL_LINES};
pub use projector::{fold_runs, RunRow};

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error("worker record is schema version {0}, newer than this build understands")]
    UnknownVersion(u32),
    #[error("{0}")]
    Malformed(String),
}

pub type Result<T> = std::result::Result<T, WorkerError>;
