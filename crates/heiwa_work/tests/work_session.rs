use heiwa_evidence::{
    CursorEvent, OperatorActor, OperatorEvent, OperatorEventType, OperatorRisk,
    OperatorSensitivity, OPERATOR_EVENT_SCHEMA_VERSION,
};
use heiwa_work::{
    build_work_session, work_created_event, ProjectionEpoch, WorkId, WorkSessionBuildError,
    WorkSessionBuildOptions,
};
use serde_json::json;

fn event(
    event_id: &str,
    work_id: &str,
    thread_id: &str,
    turn_id: Option<&str>,
    call_id: Option<&str>,
    event_type: OperatorEventType,
    payload: serde_json::Value,
) -> OperatorEvent {
    OperatorEvent {
        schema_version: OPERATOR_EVENT_SCHEMA_VERSION,
        event_id: event_id.to_string(),
        thread_id: thread_id.to_string(),
        turn_id: turn_id.map(str::to_string),
        run_id: None,
        call_id: call_id.map(str::to_string),
        work_id: Some(work_id.to_string()),
        event_type,
        occurred_at: format!("2026-08-25T00:00:{:02}Z", event_id.len()),
        actor: OperatorActor {
            kind: "runtime".to_string(),
            id: "test".to_string(),
        },
        risk_class: OperatorRisk::Low,
        sensitivity: OperatorSensitivity::LocalPrivate,
        parent_event_id: None,
        correlation_id: call_id.map(str::to_string),
        source_refs: vec![],
        evidence_refs: vec![],
        payload,
    }
}

fn rows() -> Vec<CursorEvent> {
    let work_id = WorkId::parse("work-abc").unwrap();
    let mut events = vec![work_created_event(
        &work_id,
        "thread-1",
        "ship the Work-bound execution slice",
        "installation-1",
        "2026-08-25T00:00:00Z",
        || "evt-created".to_string(),
    )];
    events.extend([
        event(
            "evt-workspace",
            "work-abc",
            "thread-1",
            None,
            None,
            OperatorEventType::WorkspacePrepared,
            json!({
                "repo_root": "/repo",
                "worktree_path": "/repo/.worktrees/work-abc",
                "branch": "heiwa/work-abc",
                "base_commit": "abc123"
            }),
        ),
        event(
            "evt-turn",
            "work-abc",
            "thread-1",
            Some("turn-1"),
            None,
            OperatorEventType::TurnStarted,
            json!({"client_request_id": "request-1", "prompt_fingerprint": "digest"}),
        ),
        event(
            "evt-approval",
            "work-abc",
            "thread-1",
            Some("turn-1"),
            Some("call-1"),
            OperatorEventType::ApprovalDecided,
            json!({
                "request_id": "approval-1",
                "tool": "fs.write",
                "risk": "high",
                "outcome": "approved"
            }),
        ),
        event(
            "evt-tool",
            "work-abc",
            "thread-1",
            Some("turn-1"),
            Some("call-1"),
            OperatorEventType::ToolCallCompleted,
            json!({
                "name": "fs.write",
                "arguments": {"secret": "must-not-project"},
                "status": "success",
                "output": "must-not-project",
                "receipt_id": "receipt-tool-1"
            }),
        ),
        event(
            "evt-artifact",
            "work-abc",
            "thread-1",
            Some("turn-1"),
            Some("call-1"),
            OperatorEventType::ArtifactCreated,
            json!({
                "artifact_id": "artifact-1",
                "artifact_ref": "artifact://artifact-1",
                "kind": "test_report",
                "byte_len": 42,
                "body": "must-not-project"
            }),
        ),
        event(
            "evt-receipt",
            "work-abc",
            "thread-1",
            Some("turn-1"),
            None,
            OperatorEventType::ReceiptLinked,
            json!({
                "kind": "operator_turn",
                "receipt_ref": "receipt-turn-1",
                "provider": "test",
                "model": "test-model",
                "cost_usd": 0.0
            }),
        ),
        event(
            "evt-complete",
            "work-abc",
            "thread-1",
            Some("turn-1"),
            None,
            OperatorEventType::TurnCompleted,
            json!({"trace": {"private": "must-not-project"}}),
        ),
        event(
            "evt-foreign",
            "work-def",
            "thread-2",
            Some("turn-2"),
            None,
            OperatorEventType::Blocker,
            json!({"reason": "foreign"}),
        ),
    ]);
    events
        .into_iter()
        .enumerate()
        .map(|(index, event)| CursorEvent {
            cursor: format!("cursor-{index}"),
            event,
        })
        .collect()
}

#[test]
fn work_session_rejects_an_unknown_work() {
    let error = build_work_session(
        &rows(),
        "work-missing",
        WorkSessionBuildOptions::new("fold-1", 32),
    )
    .unwrap_err();
    assert_eq!(
        error,
        WorkSessionBuildError::UnknownWork("work-missing".to_string())
    );
}

#[test]
fn work_session_projects_one_work_without_sensitive_bodies() {
    let snapshot = build_work_session(
        &rows(),
        "work-abc",
        WorkSessionBuildOptions::new("fold-1", 32),
    )
    .unwrap();

    assert_eq!(snapshot.work_id, "work-abc");
    assert_eq!(snapshot.work_revision, 1);
    assert_eq!(
        snapshot.projection_epoch,
        ProjectionEpoch::from_seed("fold-1")
    );
    assert_eq!(snapshot.projection_revision, 8);
    assert_eq!(snapshot.operator_cursor.as_deref(), Some("cursor-8"));
    for collection in [
        "work",
        "threads",
        "workspace",
        "approvals",
        "actions",
        "artifacts",
        "receipts",
    ] {
        assert!(
            snapshot.collections.contains_key(collection),
            "{collection}"
        );
    }
    assert_eq!(
        snapshot.collections["threads"]["thread-1"]["status"],
        "completed"
    );
    assert_eq!(
        snapshot.collections["actions"]["call-1"]["status"],
        "success"
    );
    assert_eq!(
        snapshot.collections["workspace"]["/repo"]["branch"],
        "heiwa/work-abc"
    );
    let encoded = serde_json::to_string(&snapshot).unwrap();
    assert!(!encoded.contains("must-not-project"), "{encoded}");
    assert!(!encoded.contains("work-def"), "{encoded}");
}

#[test]
fn work_session_bounds_unique_rows_and_reports_truncation() {
    let mut bounded_rows = rows();
    for index in 0..3 {
        bounded_rows.push(CursorEvent {
            cursor: format!("cursor-extra-{index}"),
            event: event(
                &format!("evt-artifact-extra-{index}"),
                "work-abc",
                "thread-1",
                Some("turn-1"),
                None,
                OperatorEventType::ArtifactCreated,
                json!({
                    "artifact_id": format!("artifact-extra-{index}"),
                    "kind": "test_report",
                    "byte_len": index
                }),
            ),
        });
    }

    let snapshot = build_work_session(
        &bounded_rows,
        "work-abc",
        WorkSessionBuildOptions::new("fold-1", 2),
    )
    .unwrap();

    assert_eq!(snapshot.collections["artifacts"].len(), 2);
    assert_eq!(snapshot.truncated_collections["artifacts"], 2);
    assert_eq!(
        snapshot.operator_cursor.as_deref(),
        Some("cursor-extra-2"),
        "the stream boundary advances across rows omitted from bounded collections"
    );
}

#[test]
fn work_session_does_not_render_cancel_audit_as_running() {
    let mut cancel_rows = rows().into_iter().take(3).collect::<Vec<_>>();
    for (cursor, event) in [
        (
            "cursor-cancel",
            event(
                "evt-cancel",
                "work-abc",
                "thread-1",
                Some("turn-1"),
                None,
                OperatorEventType::TurnCancelRequested,
                json!({"reason": "OPERATOR_REQUEST"}),
            ),
        ),
        (
            "cursor-cancel-audit",
            event(
                "evt-cancel-audit",
                "work-abc",
                "thread-1",
                Some("turn-1"),
                Some("call-1"),
                OperatorEventType::ApprovalDecided,
                json!({"request_id": "approval-1", "outcome": "cancelled"}),
            ),
        ),
    ] {
        cancel_rows.push(CursorEvent {
            cursor: cursor.to_string(),
            event,
        });
    }

    let snapshot = build_work_session(
        &cancel_rows,
        "work-abc",
        WorkSessionBuildOptions::new("fold-cancel", 32),
    )
    .unwrap();
    assert_eq!(
        snapshot.collections["threads"]["thread-1"]["status"],
        "cancelling"
    );
}
