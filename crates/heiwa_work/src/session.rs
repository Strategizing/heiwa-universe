//! Canonical bounded read model for one Work session.
//!
//! This is a pure projection over the append-ordered operator stream. It does
//! not write, fetch, or open a second store, and it intentionally excludes raw
//! prompts, tool arguments/output, artifact bodies, and full diffs.

use std::collections::BTreeMap;

use heiwa_evidence::{CursorEvent, OperatorEvent, OperatorEventType};
use serde_json::{json, Value};

use crate::{fold, CollectionRows, ProjectionEpoch, WorkSessionSnapshotV1};

const COLLECTIONS: [&str; 10] = [
    "work",
    "threads",
    "workspace",
    "runs",
    "approvals",
    "actions",
    "artifacts",
    "tests",
    "receipts",
    "blockers",
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkSessionBuildOptions {
    pub epoch_seed: String,
    pub collection_limit: usize,
}

impl WorkSessionBuildOptions {
    pub fn new(epoch_seed: impl Into<String>, collection_limit: usize) -> Self {
        Self {
            epoch_seed: epoch_seed.into(),
            collection_limit: collection_limit.max(1),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WorkSessionBuildError {
    #[error("unknown Work {0}")]
    UnknownWork(String),
}

pub fn build_work_session(
    rows: &[CursorEvent],
    work_id: &str,
    options: WorkSessionBuildOptions,
) -> Result<WorkSessionSnapshotV1, WorkSessionBuildError> {
    let events = rows.iter().map(|row| row.event.clone()).collect::<Vec<_>>();
    let projection = fold(&events);
    let work = projection
        .work(work_id)
        .ok_or_else(|| WorkSessionBuildError::UnknownWork(work_id.to_string()))?;

    let mut collections = COLLECTIONS
        .into_iter()
        .map(|name| (name.to_string(), CollectionRows::default()))
        .collect::<BTreeMap<_, _>>();
    let mut truncated = BTreeMap::new();
    bounded_upsert(
        &mut collections,
        &mut truncated,
        "work",
        work_id,
        json!({
            "work_id": work.work_id.as_str(),
            "intent": work.intent,
            "status": work.status,
            "revision": work.revision,
            "primary_thread_id": work.primary_thread_id,
            "related_thread_ids": work.related_thread_ids,
            "updated_at": work.updated_at,
        }),
        options.collection_limit,
    );

    let mut projection_revision = 0_u64;
    for row in rows {
        let event = &row.event;
        if event.work_id.as_deref() != Some(work_id) {
            continue;
        }
        projection_revision = projection_revision.saturating_add(1);
        project_event(
            &mut collections,
            &mut truncated,
            event,
            options.collection_limit,
        );
    }

    // Runs fold separately because a run row is assembled from several events
    // (launch, heartbeat, exit, pane open/close) rather than upserted per
    // event. `projection_revision` still counts events, not rows.
    for row in heiwa_worker::fold_runs(&events, work_id) {
        let run_id = row.run_id.clone();
        let value = serde_json::to_value(&row).expect("run row is plain data");
        bounded_upsert(
            &mut collections,
            &mut truncated,
            "runs",
            &run_id,
            value,
            options.collection_limit,
        );
    }

    let operator_cursor = rows.last().map(|row| row.cursor.clone());
    let mut source_watermarks = BTreeMap::new();
    if let Some(cursor) = &operator_cursor {
        source_watermarks.insert("operator".to_string(), cursor.clone());
    }

    Ok(WorkSessionSnapshotV1 {
        work_id: work_id.to_string(),
        work_revision: work.revision,
        projection_epoch: ProjectionEpoch::from_seed(&options.epoch_seed),
        projection_revision,
        operator_cursor,
        source_watermarks,
        collections,
        truncated_collections: truncated,
    })
}

fn project_event(
    collections: &mut BTreeMap<String, CollectionRows>,
    truncated: &mut BTreeMap<String, usize>,
    event: &OperatorEvent,
    limit: usize,
) {
    project_thread(collections, truncated, event, limit);
    match event.event_type {
        OperatorEventType::WorkspacePrepared => {
            let Some(repo_root) = string(&event.payload, "repo_root") else {
                return;
            };
            bounded_upsert(
                collections,
                truncated,
                "workspace",
                repo_root,
                json!({
                    "repo_root": repo_root,
                    "worktree_path": string(&event.payload, "worktree_path"),
                    "branch": string(&event.payload, "branch"),
                    "base_commit": string(&event.payload, "base_commit"),
                    "state": "prepared",
                    "updated_at": event.occurred_at,
                }),
                limit,
            );
        }
        OperatorEventType::WorkspaceReleased => {
            let Some(repo_root) = string(&event.payload, "repo_root") else {
                return;
            };
            bounded_upsert(
                collections,
                truncated,
                "workspace",
                repo_root,
                json!({
                    "repo_root": repo_root,
                    "state": "released",
                    "updated_at": event.occurred_at,
                }),
                limit,
            );
        }
        OperatorEventType::ApprovalRequested | OperatorEventType::ApprovalDecided => {
            let key = string(&event.payload, "request_id")
                .or(event.call_id.as_deref())
                .unwrap_or(&event.event_id);
            bounded_upsert(
                collections,
                truncated,
                "approvals",
                key,
                json!({
                    "request_id": string(&event.payload, "request_id"),
                    "call_id": event.call_id,
                    "tool": string(&event.payload, "tool"),
                    "risk": string(&event.payload, "risk"),
                    "outcome": string(&event.payload, "outcome"),
                    "updated_at": event.occurred_at,
                }),
                limit,
            );
        }
        OperatorEventType::ToolCallStarted | OperatorEventType::ToolCallCompleted => {
            let key = event.call_id.as_deref().unwrap_or(&event.event_id);
            let status = if event.event_type == OperatorEventType::ToolCallStarted {
                "started"
            } else {
                string(&event.payload, "status").unwrap_or("completed")
            };
            bounded_upsert(
                collections,
                truncated,
                "actions",
                key,
                json!({
                    "call_id": event.call_id,
                    "name": string(&event.payload, "name"),
                    "status": status,
                    "receipt_id": string(&event.payload, "receipt_id"),
                    "artifact_ref": string(&event.payload, "artifact_ref"),
                    "updated_at": event.occurred_at,
                }),
                limit,
            );
        }
        OperatorEventType::ArtifactCreated => {
            let key = string(&event.payload, "artifact_id").unwrap_or(&event.event_id);
            bounded_upsert(
                collections,
                truncated,
                "artifacts",
                key,
                json!({
                    "artifact_id": string(&event.payload, "artifact_id"),
                    "artifact_ref": string(&event.payload, "artifact_ref"),
                    "kind": string(&event.payload, "kind"),
                    "byte_len": event.payload.get("byte_len").and_then(Value::as_u64),
                    "created_at": event.occurred_at,
                }),
                limit,
            );
        }
        OperatorEventType::TestResult => {
            bounded_upsert(
                collections,
                truncated,
                "tests",
                &event.event_id,
                json!({
                    "event_id": event.event_id,
                    "name": string(&event.payload, "name"),
                    "status": string(&event.payload, "status"),
                    "occurred_at": event.occurred_at,
                }),
                limit,
            );
        }
        OperatorEventType::ReceiptLinked => {
            bounded_upsert(
                collections,
                truncated,
                "receipts",
                &event.event_id,
                json!({
                    "event_id": event.event_id,
                    "kind": string(&event.payload, "kind"),
                    "receipt_ref": string(&event.payload, "receipt_ref"),
                    "provider": string(&event.payload, "provider"),
                    "model": string(&event.payload, "model"),
                    "cost_usd": event.payload.get("cost_usd").and_then(Value::as_f64),
                    "occurred_at": event.occurred_at,
                }),
                limit,
            );
        }
        OperatorEventType::Blocker => {
            bounded_upsert(
                collections,
                truncated,
                "blockers",
                &event.event_id,
                json!({
                    "event_id": event.event_id,
                    "code": string(&event.payload, "code"),
                    "reason": string(&event.payload, "reason").map(|value| bounded(value, 256)),
                    "occurred_at": event.occurred_at,
                }),
                limit,
            );
        }
        _ => {}
    }
}

fn project_thread(
    collections: &mut BTreeMap<String, CollectionRows>,
    truncated: &mut BTreeMap<String, usize>,
    event: &OperatorEvent,
    limit: usize,
) {
    let status = match event.event_type {
        OperatorEventType::TurnStarted => Some("open"),
        OperatorEventType::TurnCancelRequested => Some("cancelling"),
        OperatorEventType::TurnCompleted => Some("completed"),
        OperatorEventType::TurnInterrupted => Some("interrupted"),
        OperatorEventType::Blocker if event.turn_id.is_some() => Some("blocked"),
        _ if event.turn_id.is_some() => {
            let current = collections
                .get("threads")
                .and_then(|rows| rows.get(&event.thread_id))
                .and_then(|row| row.get("status"))
                .and_then(Value::as_str);
            Some(if current == Some("cancelling") {
                "cancelling"
            } else {
                "running"
            })
        }
        _ => None,
    };
    let Some(status) = status else {
        return;
    };
    bounded_upsert(
        collections,
        truncated,
        "threads",
        &event.thread_id,
        json!({
            "thread_id": event.thread_id,
            "latest_turn_id": event.turn_id,
            "status": status,
            "updated_at": event.occurred_at,
        }),
        limit,
    );
}

fn bounded_upsert(
    collections: &mut BTreeMap<String, CollectionRows>,
    truncated: &mut BTreeMap<String, usize>,
    collection: &str,
    key: &str,
    value: Value,
    limit: usize,
) {
    let rows = collections
        .get_mut(collection)
        .expect("known Work-session collection");
    if rows.contains_key(key) || rows.len() < limit {
        rows.insert(key.to_string(), value);
    } else {
        *truncated.entry(collection.to_string()).or_default() += 1;
    }
}

fn string<'a>(payload: &'a Value, key: &str) -> Option<&'a str> {
    payload.get(key).and_then(Value::as_str)
}

fn bounded(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}
