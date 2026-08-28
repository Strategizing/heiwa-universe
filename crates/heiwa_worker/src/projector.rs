//! Pure fold from the operator stream to the `runs` read model.
//!
//! Append order is authority. A row is keyed by per-invocation `run_id` and only ever
//! created by a `worker_launched` envelope scoped to the requested Work — a
//! pane alone never mints one, because the spec forbids treating a terminal
//! pane as a verified worker merely because it exists.

use std::collections::BTreeMap;

use heiwa_evidence::{OperatorEvent, OperatorEventType};
use serde::{Deserialize, Serialize};

use crate::events::{
    PaneClosedPayload, PaneOpenedPayload, WorkerExitedPayload, WorkerLaunchedPayload,
};
use crate::model::{PaneState, WorkerState};

/// One worker run inside one Work, with the pane bound to it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRow {
    pub run_id: String,
    pub worker_id: String,
    pub work_id: String,
    pub worker_state: WorkerState,
    pub provider: Option<String>,
    pub provider_session_ref: Option<String>,
    pub executable_path: Option<String>,
    pub executable_sha256: Option<String>,
    pub cwd: Option<String>,
    pub repo_root: Option<String>,
    pub branch: Option<String>,
    pub base_commit: Option<String>,
    pub lease_id: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub exit_code: Option<i32>,
    pub failure_code: Option<String>,
    pub pane_id: Option<String>,
    pub pane_state: Option<PaneState>,
    pub pane_tail: Vec<String>,
    pub pane_dropped_lines: usize,
}

impl RunRow {
    fn new(run_id: String, worker_id: String, work_id: String) -> Self {
        Self {
            worker_id,
            run_id,
            work_id,
            worker_state: WorkerState::Starting,
            provider: None,
            provider_session_ref: None,
            executable_path: None,
            executable_sha256: None,
            cwd: None,
            repo_root: None,
            branch: None,
            base_commit: None,
            lease_id: None,
            started_at: None,
            ended_at: None,
            exit_code: None,
            failure_code: None,
            pane_id: None,
            pane_state: None,
            pane_tail: Vec::new(),
            pane_dropped_lines: 0,
        }
    }
}

/// Fold `events` into the run rows belonging to `work_id`, in first-launch
/// order.
pub fn fold_runs(events: &[OperatorEvent], work_id: &str) -> Vec<RunRow> {
    let mut rows: BTreeMap<String, RunRow> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();

    for event in events {
        if event.work_id.as_deref() != Some(work_id) {
            continue;
        }
        match event.event_type {
            OperatorEventType::WorkerLaunched => {
                // The envelope's run_id is one execution; the payload carries
                // the stable worker that owns the prepared lease. A payload
                // that no longer deserializes must not
                // make the run vanish — the envelope already said it exists.
                let Some(run_id) = event.run_id.clone() else {
                    continue;
                };
                if !rows.contains_key(&run_id) {
                    order.push(run_id.clone());
                    rows.insert(
                        run_id.clone(),
                        RunRow::new(run_id.clone(), event.actor.id.clone(), work_id.to_string()),
                    );
                }
                let row = rows.get_mut(&run_id).expect("just inserted");
                row.started_at = Some(event.occurred_at.clone());
                if let Some(payload) = WorkerLaunchedPayload::from_event(event) {
                    row.worker_id = payload.worker_id;
                    row.provider = Some(payload.provider);
                    row.provider_session_ref = payload.provider_session_ref;
                    row.executable_path = Some(payload.executable_path);
                    row.executable_sha256 = Some(payload.executable_sha256);
                    row.cwd = Some(payload.cwd);
                    row.repo_root = Some(payload.repo_root);
                    row.branch = Some(payload.branch);
                    row.base_commit = Some(payload.base_commit);
                    row.lease_id = Some(payload.lease_id);
                }
            }
            OperatorEventType::WorkerHeartbeat => {
                if let Some(id) = event.run_id.as_deref() {
                    if let Some(row) = rows.get_mut(id) {
                        if row.worker_state == WorkerState::Starting {
                            row.worker_state = WorkerState::Live;
                        }
                    }
                }
            }
            OperatorEventType::WorkerExited => {
                let Some(id) = event.run_id.as_deref() else {
                    continue;
                };
                let Some(row) = rows.get_mut(id) else {
                    continue;
                };
                row.ended_at = Some(event.occurred_at.clone());
                if let Some(payload) = WorkerExitedPayload::from_event(event) {
                    row.exit_code = payload.exit_code;
                    row.failure_code = payload.failure_code.clone();
                    row.worker_state = if payload.failure_code.is_some() {
                        WorkerState::Failed
                    } else {
                        WorkerState::Exited
                    };
                } else {
                    row.worker_state = WorkerState::Exited;
                }
                if row.pane_id.is_some() {
                    row.pane_state = Some(PaneState::for_worker(row.worker_state));
                }
            }
            OperatorEventType::PaneOpened => {
                let Some(payload) = PaneOpenedPayload::from_event(event) else {
                    continue;
                };
                // A pane never mints a run: an unlaunched worker stays unknown.
                let Some(run_id) = event.run_id.as_deref() else {
                    continue;
                };
                let Some(row) = rows.get_mut(run_id) else {
                    continue;
                };
                row.pane_id = Some(payload.pane_id);
                row.pane_state = Some(PaneState::for_worker(row.worker_state));
            }
            OperatorEventType::PaneClosed => {
                let Some(payload) = PaneClosedPayload::from_event(event) else {
                    continue;
                };
                let Some(run_id) = event.run_id.as_deref() else {
                    continue;
                };
                let Some(row) = rows.get_mut(run_id) else {
                    continue;
                };
                row.pane_tail = payload.tail;
                row.pane_dropped_lines = payload.dropped_lines;
                row.pane_state = Some(PaneState::for_worker(row.worker_state));
            }
            _ => {}
        }
    }

    order
        .into_iter()
        .filter_map(|id| rows.remove(&id))
        .collect()
}
