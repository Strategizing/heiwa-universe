//! Materialization, idempotency, validation, and recovery tests for
//! [`heiwa_session::operator::OperatorSessionService`].
//!
//! Every test builds its own `OperatorJournal` under a fresh `tempfile`
//! directory and never touches `HOME` or the real `~/.heiwa` corpus.

use heiwa_evidence::{
    OperatorActor, OperatorEvent, OperatorEventType, OperatorJournal, OperatorRisk,
    OperatorSensitivity, OPERATOR_EVENT_SCHEMA_VERSION,
};
use heiwa_session::operator::{
    OperatorAppRuntimeLease, OperatorOwnershipError, OperatorSessionService, RouteMode,
    StartTurnRequest, TurnSubmissionError,
};
#[cfg(feature = "lance")]
use heiwa_session::{operator_event_key, SessionSearchHit};
use heiwa_session::{rebuild_operator_indexes_at, EmbeddingSink};
use serde_json::json;
#[cfg(feature = "lance")]
use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::sync::{Arc, Barrier, Mutex};

fn test_service(path: &std::path::Path) -> OperatorSessionService {
    OperatorSessionService::new(OperatorJournal::new(path.to_path_buf()).unwrap())
}

fn create_work(service: &OperatorSessionService, work_id: &str, thread_id: &str) {
    service.ensure_thread(thread_id).expect("thread");
    let mut created = base_event(thread_id, None, None, OperatorEventType::WorkCreated);
    created.work_id = Some(work_id.to_string());
    created.payload = json!({
        "intent": "exercise Work-scoped execution",
        "origin_installation_id": "installation-test",
        "primary_thread_id": thread_id,
    });
    service.append_event(created).expect("work created");
}

fn work_request(request_id: &str, work_id: &str) -> StartTurnRequest {
    let mut request = StartTurnRequest::auto(request_id, "ship it");
    request.work_id = Some(work_id.to_string());
    request
}

#[derive(Default)]
struct RecordingEmbedder {
    rows: Mutex<Vec<(String, String, String)>>,
}

#[derive(Default)]
struct StreamingOnlyEmbedder {
    rows: Mutex<Vec<(String, String, String)>>,
    lifecycle: Mutex<Vec<&'static str>>,
}

impl EmbeddingSink for StreamingOnlyEmbedder {
    fn begin_replace(&self) -> anyhow::Result<()> {
        self.lifecycle.lock().unwrap().push("begin");
        self.rows.lock().unwrap().clear();
        Ok(())
    }

    fn upsert_text(&self, thread_id: &str, event_id: &str, text: &str) -> anyhow::Result<bool> {
        self.rows.lock().unwrap().push((
            thread_id.to_string(),
            event_id.to_string(),
            text.to_string(),
        ));
        Ok(true)
    }

    fn finalize_replace(&self) -> anyhow::Result<()> {
        self.lifecycle.lock().unwrap().push("finalize");
        Ok(())
    }
}

#[cfg(feature = "lance")]
struct DeterministicLanceSink {
    store: Mutex<heiwa_embed::LanceVectorStore>,
}

#[cfg(feature = "lance")]
impl DeterministicLanceSink {
    fn open(path: &std::path::Path) -> Self {
        Self {
            store: Mutex::new(heiwa_embed::LanceVectorStore::open(path, 4).unwrap()),
        }
    }

    fn insert_stale(&self, thread_id: &str, event_id: &str) {
        self.store
            .lock()
            .unwrap()
            .upsert(
                thread_id,
                operator_event_key(event_id),
                "deterministic-test",
                &[1.0, 0.0, 0.0, 0.0],
            )
            .unwrap();
    }

    fn matching_event_ids(
        &self,
        thread_id: &str,
        event_ids_by_key: &HashMap<u64, String>,
    ) -> HashSet<String> {
        self.store
            .lock()
            .unwrap()
            .top_k_similar(thread_id, &[1.0, 0.0, 0.0, 0.0], 100)
            .unwrap()
            .into_iter()
            .map(|hit| {
                event_ids_by_key
                    .get(&hit.entry_id)
                    .expect("every Lance hit must join to a known event id")
                    .clone()
            })
            .collect()
    }
}

#[cfg(feature = "lance")]
impl EmbeddingSink for DeterministicLanceSink {
    fn begin_replace(&self) -> anyhow::Result<()> {
        self.store
            .lock()
            .unwrap()
            .rebuild_from(std::iter::empty::<heiwa_embed::EmbeddingRow>())?;
        Ok(())
    }

    fn upsert_text(&self, thread_id: &str, event_id: &str, _text: &str) -> anyhow::Result<bool> {
        self.store.lock().unwrap().upsert(
            thread_id,
            operator_event_key(event_id),
            "deterministic-test",
            &[1.0, 0.0, 0.0, 0.0],
        )?;
        Ok(true)
    }
}

#[cfg(feature = "lance")]
fn fts_event_ids(index_path: &std::path::Path, query: &str) -> HashSet<String> {
    heiwa_session::operator_index::search_session_messages_at(
        index_path,
        Some("default"),
        query,
        100,
    )
    .unwrap()
    .into_iter()
    .map(|hit: SessionSearchHit| hit.event_id)
    .collect()
}

impl EmbeddingSink for RecordingEmbedder {
    fn begin_replace(&self) -> anyhow::Result<()> {
        self.rows.lock().unwrap().clear();
        Ok(())
    }

    fn upsert_text(&self, thread_id: &str, event_id: &str, text: &str) -> anyhow::Result<bool> {
        self.rows.lock().unwrap().push((
            thread_id.to_string(),
            event_id.to_string(),
            text.to_string(),
        ));
        Ok(true)
    }
}

#[test]
fn rebuild_indexes_projects_safe_text_and_only_embeds_messages() {
    let evidence = tempfile::tempdir().unwrap();
    let indexes = tempfile::tempdir().unwrap();
    let service = test_service(evidence.path());
    let turn = service
        .start_turn(
            "default",
            StartTurnRequest::auto("request-1", "index this user text"),
        )
        .unwrap();
    let mut assistant = base_event(
        "default",
        Some(&turn.turn_id),
        None,
        OperatorEventType::AssistantCompleted,
    );
    assistant.payload = json!({"text": "index this assistant text"});
    service.append_event(assistant).unwrap();
    let mut tool_started = base_event(
        "default",
        Some(&turn.turn_id),
        Some("call-1"),
        OperatorEventType::ToolCallStarted,
    );
    tool_started.payload = json!({"name": "shell"});
    service.append_event(tool_started).unwrap();
    let mut tool = base_event(
        "default",
        Some(&turn.turn_id),
        Some("call-1"),
        OperatorEventType::ToolCallCompleted,
    );
    tool.sensitivity = OperatorSensitivity::Restricted;
    tool.payload = json!({"name": "shell", "output": "restricted but safe tool output"});
    service.append_event(tool).unwrap();

    let sink = RecordingEmbedder::default();
    let first =
        rebuild_operator_indexes_at(&service, &sink, &indexes.path().join("sessions.sqlite3"))
            .unwrap();
    let second =
        rebuild_operator_indexes_at(&service, &sink, &indexes.path().join("sessions.sqlite3"))
            .unwrap();

    assert_eq!(first.fts_rows, 3);
    assert_eq!(first.embedded_rows, 2);
    assert_eq!(first.embedding_failures, 0);
    assert_eq!(second, first);
    assert_eq!(
        sink.rows.lock().unwrap().len(),
        2,
        "replacement removes stale embedding rows before each rebuild"
    );
}

#[test]
fn rebuild_indexes_streams_embedding_rows_without_bulk_materialization() {
    let evidence = tempfile::tempdir().unwrap();
    let indexes = tempfile::tempdir().unwrap();
    let service = test_service(evidence.path());
    for index in 0..300 {
        service
            .start_turn(
                "default",
                StartTurnRequest::auto(
                    format!("bounded-{index}"),
                    format!("bounded index message {index}"),
                ),
            )
            .unwrap();
    }

    let sink = StreamingOnlyEmbedder::default();
    let report =
        rebuild_operator_indexes_at(&service, &sink, &indexes.path().join("sessions.sqlite3"))
            .unwrap();

    assert_eq!(report.fts_rows, 300);
    assert_eq!(report.embedded_rows, 300);
    assert_eq!(report.embedding_failures, 0);
    assert_eq!(sink.rows.lock().unwrap().len(), 300);
    assert_eq!(
        sink.lifecycle.lock().unwrap().as_slice(),
        ["begin", "finalize"]
    );
}

#[test]
fn rebuild_indexes_deduplicates_event_ids_before_projecting_message_text() {
    let evidence = tempfile::tempdir().unwrap();
    let indexes = tempfile::tempdir().unwrap();
    let service = test_service(evidence.path());
    let turn = service
        .start_turn(
            "default",
            StartTurnRequest::auto("dedup-index-request", "canonical message"),
        )
        .unwrap();

    let duplicate_id = "duplicate-across-event-kinds";
    let mut first = base_event(
        "default",
        Some(&turn.turn_id),
        None,
        OperatorEventType::AssistantStarted,
    );
    first.event_id = duplicate_id.to_string();
    service.append_event(first).unwrap();

    let mut repeated = base_event(
        "default",
        Some(&turn.turn_id),
        None,
        OperatorEventType::AssistantCompleted,
    );
    repeated.event_id = duplicate_id.to_string();
    repeated.payload = json!({"text": "must not enter derived indexes"});
    service.append_event(repeated).unwrap();

    let sink = RecordingEmbedder::default();
    let index_path = indexes.path().join("sessions.sqlite3");
    let report = rebuild_operator_indexes_at(&service, &sink, &index_path).unwrap();

    assert_eq!(report.fts_rows, 1);
    assert_eq!(report.embedded_rows, 1);
    assert!(heiwa_session::operator_index::search_session_messages_at(
        &index_path,
        Some("default"),
        "\"derived indexes\"",
        10,
    )
    .unwrap()
    .is_empty());
}

#[cfg(feature = "lance")]
#[test]
fn deleting_derived_indexes_rebuilds_identical_fts_and_lance_event_sets_from_journal() {
    let root = tempfile::tempdir().unwrap();
    let evidence_path = root.path().join("evidence");
    let fts_path = root.path().join("indexes/sessions.sqlite3");
    let lance_path = root.path().join("indexes/lance");
    let service = test_service(&evidence_path);
    let turn = service
        .start_turn(
            "default",
            StartTurnRequest::auto("rebuild-request", "phoenix rebuild proof from user"),
        )
        .unwrap();
    let mut assistant = base_event(
        "default",
        Some(&turn.turn_id),
        None,
        OperatorEventType::AssistantCompleted,
    );
    assistant.payload = json!({"text": "phoenix rebuild proof from assistant"});
    service.append_event(assistant).unwrap();

    let expected_ids = service
        .events_after("default", None, 100)
        .unwrap()
        .events
        .into_iter()
        .filter(|row| {
            matches!(
                row.event.event_type,
                OperatorEventType::UserMessage | OperatorEventType::AssistantCompleted
            )
        })
        .map(|row| row.event.event_id)
        .collect::<HashSet<_>>();
    let mut event_ids_by_key = expected_ids
        .iter()
        .map(|event_id| (operator_event_key(event_id), event_id.clone()))
        .collect::<HashMap<_, _>>();
    let stale_event_id = "stale-event-not-in-journal";
    event_ids_by_key.insert(
        operator_event_key(stale_event_id),
        stale_event_id.to_string(),
    );
    assert_eq!(expected_ids.len(), 2);
    assert_eq!(event_ids_by_key.len(), expected_ids.len() + 1);

    let sink = DeterministicLanceSink::open(&lance_path);
    rebuild_operator_indexes_at(&service, &sink, &fts_path).unwrap();
    let mut contaminated_ids = expected_ids.clone();
    contaminated_ids.insert(stale_event_id.to_string());
    {
        let conn = rusqlite::Connection::open(&fts_path).unwrap();
        conn.execute(
            "INSERT INTO messages_fts (thread_id, event_id, entry_id, role, content)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                "default",
                stale_event_id,
                operator_event_key(stale_event_id) as i64,
                "assistant",
                "phoenix stale projection"
            ],
        )
        .unwrap();
    }
    sink.insert_stale("default", stale_event_id);
    assert_eq!(fts_event_ids(&fts_path, "phoenix"), contaminated_ids);
    assert_eq!(
        sink.matching_event_ids("default", &event_ids_by_key),
        contaminated_ids
    );

    let before_report = rebuild_operator_indexes_at(&service, &sink, &fts_path).unwrap();
    let fts_before = fts_event_ids(&fts_path, "phoenix");
    let lance_before = sink.matching_event_ids("default", &event_ids_by_key);
    assert_eq!(before_report.fts_rows, expected_ids.len());
    assert_eq!(before_report.embedded_rows, expected_ids.len());
    assert_eq!(fts_before, expected_ids);
    assert_eq!(lance_before, expected_ids);
    assert!(!fts_before.contains(stale_event_id));
    assert!(!lance_before.contains(stale_event_id));
    drop(sink);

    std::fs::remove_file(&fts_path).unwrap();
    std::fs::remove_dir_all(&lance_path).unwrap();

    let rebuilt_sink = DeterministicLanceSink::open(&lance_path);
    let after_report = rebuild_operator_indexes_at(&service, &rebuilt_sink, &fts_path).unwrap();
    let fts_after = fts_event_ids(&fts_path, "phoenix");
    let lance_after = rebuilt_sink.matching_event_ids("default", &event_ids_by_key);
    assert_eq!(after_report, before_report);
    assert_eq!(fts_after, fts_before);
    assert_eq!(lance_after, lance_before);
    assert!(!fts_after.contains(stale_event_id));
    assert!(!lance_after.contains(stale_event_id));
}

/// Build a syntactically-valid `OperatorEvent` for validation tests: correct
/// schema version, a fixed occurred_at, and a fresh random event_id so
/// repeated calls never collide. Callers override whatever field their test
/// cares about.
fn base_event(
    thread_id: &str,
    turn_id: Option<&str>,
    call_id: Option<&str>,
    event_type: OperatorEventType,
) -> OperatorEvent {
    OperatorEvent {
        schema_version: OPERATOR_EVENT_SCHEMA_VERSION,
        event_id: format!("evt-{}", uuid::Uuid::new_v4()),
        thread_id: thread_id.to_string(),
        turn_id: turn_id.map(|s| s.to_string()),
        run_id: None,
        call_id: call_id.map(|s| s.to_string()),
        work_id: None,
        event_type,
        occurred_at: "2026-07-18T00:00:00Z".to_string(),
        actor: OperatorActor {
            kind: "runtime".into(),
            id: "test-runner".into(),
        },
        risk_class: OperatorRisk::Low,
        sensitivity: OperatorSensitivity::LocalPrivate,
        parent_event_id: None,
        correlation_id: None,
        source_refs: vec![],
        evidence_refs: vec![],
        payload: json!({}),
    }
}

// ---------------------------------------------------------------------
// Step 1 verbatim: materialization / idempotency / recovery.
// ---------------------------------------------------------------------

#[test]
fn duplicate_client_request_returns_one_turn() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    let request = StartTurnRequest::auto("req-1", "hello");
    let first = service.start_turn("default", request.clone()).unwrap();
    let second = service.start_turn("default", request).unwrap();
    assert_eq!(first.turn_id, second.turn_id);
    assert!(second.duplicate);
    assert_eq!(service.thread("default").unwrap().turns.len(), 1);
}

#[test]
fn work_scoped_turn_refuses_an_unknown_work_without_writing_rows() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());

    let error = service
        .start_turn(
            "thread-1",
            work_request("work-scoped-unknown", "work-missing"),
        )
        .unwrap_err();

    assert!(
        error.to_string().contains("unknown or is not linked"),
        "{error}"
    );
    assert!(service
        .events_after("thread-1", None, 100)
        .unwrap()
        .events
        .is_empty());
    assert!(!dir.path().join("operator_events.jsonl").exists());
}

#[test]
fn work_scoped_turn_refuses_a_thread_not_linked_to_the_work() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    create_work(&service, "work-abc", "thread-primary");
    service.ensure_thread("thread-foreign").unwrap();
    let mut duplicate_creation =
        base_event("thread-foreign", None, None, OperatorEventType::WorkCreated);
    duplicate_creation.work_id = Some("work-abc".to_string());
    duplicate_creation.payload = json!({
        "intent": "must not rebind",
        "origin_installation_id": "installation-test",
        "primary_thread_id": "thread-foreign",
    });
    service.append_event(duplicate_creation).unwrap();
    let rows_before = service
        .events_after("thread-foreign", None, 100)
        .unwrap()
        .events
        .len();

    let error = service
        .start_turn(
            "thread-foreign",
            work_request("work-scoped-foreign", "work-abc"),
        )
        .unwrap_err();

    assert!(
        error.to_string().contains("unknown or is not linked"),
        "{error}"
    );
    assert_eq!(
        service
            .events_after("thread-foreign", None, 100)
            .unwrap()
            .events
            .len(),
        rows_before,
        "a duplicate creation cannot rebind Work, and refused admission appends nothing"
    );
}

#[test]
fn work_scoped_turn_appends_the_work_id_to_admission_rows() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    create_work(&service, "work-abc", "thread-primary");

    service
        .start_turn(
            "thread-primary",
            work_request("work-scoped-admitted", "work-abc"),
        )
        .unwrap();

    let rows = service
        .events_after("thread-primary", None, 100)
        .unwrap()
        .events;
    let admission = rows
        .iter()
        .filter(|row| {
            matches!(
                row.event.event_type,
                OperatorEventType::TurnStarted | OperatorEventType::UserMessage
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(admission.len(), 2);
    assert!(admission
        .iter()
        .all(|row| row.event.work_id.as_deref() == Some("work-abc")));
    assert_eq!(
        service.thread("thread-primary").unwrap().turns[0]
            .work_id
            .as_deref(),
        Some("work-abc")
    );
}

#[test]
fn work_scoped_turn_rejects_later_events_that_drop_or_change_work() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    create_work(&service, "work-abc", "thread-primary");
    let submission = service
        .start_turn(
            "thread-primary",
            work_request("work-scoped-later-events", "work-abc"),
        )
        .unwrap();

    for wrong_scope in [None, Some("work-def".to_string())] {
        let mut event = base_event(
            "thread-primary",
            Some(&submission.turn_id),
            None,
            OperatorEventType::AssistantStarted,
        );
        event.work_id = wrong_scope;
        let error = service.append_event(event).unwrap_err();
        assert!(error.to_string().contains("Work scope"), "{error}");
    }
}

#[test]
fn work_scoped_turn_accepts_a_related_thread_and_binds_retries_to_work() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    create_work(&service, "work-abc", "thread-primary");
    create_work(&service, "work-def", "thread-other");
    service.ensure_thread("thread-related").unwrap();
    let mut linked = base_event("thread-related", None, None, OperatorEventType::WorkLinked);
    linked.work_id = Some("work-abc".to_string());
    linked.payload = json!({"thread_id": "thread-related", "origin": "adopted"});
    service.append_event(linked).unwrap();

    let first = service
        .start_turn(
            "thread-related",
            work_request("work-scoped-retry", "work-abc"),
        )
        .unwrap();
    let duplicate = service
        .start_turn(
            "thread-related",
            work_request("work-scoped-retry", "work-abc"),
        )
        .unwrap();
    assert_eq!(duplicate.turn_id, first.turn_id);
    assert!(duplicate.duplicate);

    let error = service
        .start_turn(
            "thread-related",
            work_request("work-scoped-retry", "work-def"),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        TurnSubmissionError::IdempotencyConflict { .. }
    ));
    assert!(error.to_string().contains("different Work"), "{error}");
}

#[test]
fn independent_services_serialize_same_root_turn_admission() {
    let dir = tempfile::tempdir().unwrap();
    let services = [test_service(dir.path()), test_service(dir.path())];
    let barrier = Arc::new(Barrier::new(3));
    let handles = services.map(|service| {
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            barrier.wait();
            service.start_turn(
                "default",
                StartTurnRequest::auto("cross-process-request", "execute once"),
            )
        })
    });
    barrier.wait();

    let submissions = handles
        .into_iter()
        .map(|handle| handle.join().unwrap().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(submissions[0].turn_id, submissions[1].turn_id);
    assert_eq!(submissions[0].cursor, submissions[1].cursor);
    assert_eq!(
        submissions
            .iter()
            .filter(|submission| !submission.duplicate)
            .count(),
        1,
        "exactly one independent service may own execution admission"
    );
    assert_eq!(
        submissions
            .iter()
            .filter(|submission| submission.duplicate)
            .count(),
        1
    );

    let events = OperatorJournal::new(dir.path().to_path_buf())
        .unwrap()
        .read_after(None, 32)
        .unwrap()
        .events;
    for event_type in [
        OperatorEventType::ThreadCreated,
        OperatorEventType::TurnStarted,
        OperatorEventType::UserMessage,
    ] {
        assert_eq!(
            events
                .iter()
                .filter(|row| row.event.event_type == event_type)
                .count(),
            1,
            "{event_type:?} must be appended once"
        );
    }
}

#[test]
fn restart_closes_unfinished_turn_once() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    service
        .start_turn("default", StartTurnRequest::auto("req-1", "hello"))
        .unwrap();
    assert_eq!(service.recover_interrupted().unwrap(), 1);
    assert_eq!(service.recover_interrupted().unwrap(), 0);
    assert_eq!(
        service.thread("default").unwrap().turns[0].status,
        "interrupted"
    );
}

#[test]
fn restart_recovery_fails_while_another_session_writer_is_live() {
    let dir = tempfile::tempdir().unwrap();
    let live_writer = test_service(dir.path());
    live_writer
        .start_turn(
            "default",
            StartTurnRequest::auto("req-live", "still running"),
        )
        .unwrap();
    let recovery = test_service(dir.path());

    let error = recovery.recover_interrupted().unwrap_err();
    assert!(
        error.to_string().contains("operator_activity_lease_held"),
        "{error}"
    );
    assert_eq!(
        live_writer.thread("default").unwrap().turns[0].status,
        "open"
    );

    drop(live_writer);
    assert_eq!(recovery.recover_interrupted().unwrap(), 1);
    assert_eq!(recovery.recover_interrupted().unwrap(), 0);
}

#[test]
fn app_runtime_lease_is_exclusive_empty_reacquirable_and_root_scoped() {
    let first_dir = tempfile::tempdir().unwrap();
    let second_dir = tempfile::tempdir().unwrap();
    let first = OperatorAppRuntimeLease::acquire(first_dir.path()).unwrap();
    // Assert emptiness via metadata, not by reading the bytes. The lease holds
    // an exclusive lock on this file, and Windows locks are MANDATORY: any read
    // of a locked range fails with ERROR_LOCK_VIOLATION (os error 33). Unix
    // flock is advisory, so std::fs::read succeeded there and this test passed
    // on macOS and Linux while failing on every Windows runner.
    assert_eq!(
        std::fs::metadata(first_dir.path().join(".operator_runtime.lock"))
            .unwrap()
            .len(),
        0
    );
    assert!(matches!(
        OperatorAppRuntimeLease::acquire(first_dir.path()),
        Err(OperatorOwnershipError::RuntimeAlreadyHeld { .. })
    ));
    let _isolated = OperatorAppRuntimeLease::acquire(second_dir.path()).unwrap();
    drop(first);
    OperatorAppRuntimeLease::acquire(first_dir.path()).unwrap();
}

#[test]
fn read_only_service_does_not_block_restart_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let writer = test_service(dir.path());
    writer
        .start_turn("default", StartTurnRequest::auto("req-read", "recover me"))
        .unwrap();
    drop(writer);

    let reader = test_service(dir.path());
    assert_eq!(reader.thread("default").unwrap().turns[0].status, "open");
    let recovery = test_service(dir.path());
    assert_eq!(recovery.recover_interrupted().unwrap(), 1);
    assert_eq!(
        reader.thread("default").unwrap().turns[0].status,
        "interrupted"
    );
    let contender = test_service(dir.path());
    assert!(contender
        .recover_interrupted()
        .unwrap_err()
        .to_string()
        .contains("operator_activity_lease_held"));
}

#[test]
fn failed_recovery_restores_its_shared_writer_lease() {
    let dir = tempfile::tempdir().unwrap();
    let recovery = test_service(dir.path());
    recovery
        .start_turn("default", StartTurnRequest::auto("req-error", "stay open"))
        .unwrap();
    let mut stream = std::fs::OpenOptions::new()
        .append(true)
        .open(dir.path().join("operator_events.jsonl"))
        .unwrap();
    for _ in 0..1_025 {
        stream.write_all(b"not-json\n").unwrap();
    }
    drop(stream);

    let error = recovery.recover_interrupted().unwrap_err();
    assert!(error.to_string().contains("scan budget"), "{error}");
    let contender = test_service(dir.path());
    let ownership = contender.recover_interrupted().unwrap_err();
    assert!(
        ownership
            .to_string()
            .contains("operator_activity_lease_held"),
        "{ownership}"
    );
}

#[test]
fn shared_writer_lease_lives_until_last_arc_drops() {
    let dir = tempfile::tempdir().unwrap();
    let writer = std::sync::Arc::new(test_service(dir.path()));
    writer
        .start_turn("default", StartTurnRequest::auto("req-arc", "still live"))
        .unwrap();
    let last_owner = writer.clone();
    drop(writer);

    let recovery = test_service(dir.path());
    assert!(recovery.recover_interrupted().is_err());
    drop(last_owner);
    assert_eq!(recovery.recover_interrupted().unwrap(), 1);
}

#[test]
fn concurrent_recovery_attempts_append_at_most_one_terminal_event() {
    let dir = tempfile::tempdir().unwrap();
    let writer = test_service(dir.path());
    let submission = writer
        .start_turn(
            "default",
            StartTurnRequest::auto("req-concurrent-recovery", "recover once"),
        )
        .unwrap();
    drop(writer);

    let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
    let handles = (0..2)
        .map(|_| {
            let service = test_service(dir.path());
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                service.recover_interrupted()
            })
        })
        .collect::<Vec<_>>();
    barrier.wait();
    let results = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Vec<_>>();
    assert!(results.iter().any(Result::is_ok), "{results:?}");

    let journal = OperatorJournal::new(dir.path().to_path_buf()).unwrap();
    let interruptions = journal
        .read_after(None, 100)
        .unwrap()
        .events
        .iter()
        .filter(|row| {
            row.event.turn_id.as_deref() == Some(submission.turn_id.as_str())
                && row.event.event_type == OperatorEventType::TurnInterrupted
        })
        .count();
    assert_eq!(interruptions, 1, "{results:?}");
}

#[test]
fn start_turn_rejects_sensitive_prompt_before_creating_operator_events() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());

    let error = service
        .start_turn("default", StartTurnRequest::auto("req-1", "ghp_live-token"))
        .unwrap_err();
    assert!(matches!(
        &error,
        TurnSubmissionError::SensitiveMaterial { .. }
    ));
    assert!(
        error.to_string().to_lowercase().contains("sensitive"),
        "error should identify the preflight safety rejection: {error}"
    );

    assert!(
        service
            .events_after("default", None, 100)
            .unwrap()
            .events
            .is_empty(),
        "no thread_created or turn_started event may precede a rejected message"
    );
    assert!(
        !dir.path().join("operator_events.jsonl").exists(),
        "preflight must reject before creating the journal stream"
    );
}

#[test]
fn start_turn_rejects_sensitive_client_request_id_before_creating_operator_events() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());

    let error = service
        .start_turn("default", StartTurnRequest::auto("ghp_live-token", "hello"))
        .unwrap_err();
    assert!(matches!(
        error,
        TurnSubmissionError::SensitiveMaterial { .. }
    ));
    assert!(service
        .events_after("default", None, 100)
        .unwrap()
        .events
        .is_empty());
    assert!(!dir.path().join("operator_events.jsonl").exists());
}

#[test]
fn start_turn_rejects_sensitive_route_policy_before_creating_operator_events() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    let mut request = StartTurnRequest::auto("req-1", "hello");
    request.route_policy.preferred_provider = Some("ghp_live-token".to_string());

    let error = service.start_turn("default", request).unwrap_err();
    assert!(matches!(
        error,
        TurnSubmissionError::SensitiveMaterial { .. }
    ));
    assert!(service
        .events_after("default", None, 100)
        .unwrap()
        .events
        .is_empty());
    assert!(!dir.path().join("operator_events.jsonl").exists());
}

#[test]
fn orphaned_turn_retry_appends_the_missing_user_message() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    let mut orphan = base_event(
        "default",
        Some("orphan-turn"),
        None,
        OperatorEventType::TurnStarted,
    );
    orphan.payload = json!({
        "client_request_id": "req-1",
        "prompt_fingerprint": "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
        "route_policy": StartTurnRequest::auto("req-1", "hello").route_policy,
    });
    service.append_event(orphan).unwrap();

    let retry = service
        .start_turn("default", StartTurnRequest::auto("req-1", "hello"))
        .unwrap();
    assert!(retry.duplicate);
    assert_eq!(retry.turn_id, "orphan-turn");
    assert_eq!(
        service.thread("default").unwrap().turns[0]
            .prompt
            .as_deref(),
        Some("hello")
    );
}

#[test]
fn orphaned_turn_retry_without_durable_route_policy_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    let mut orphan = base_event(
        "default",
        Some("legacy-orphan-turn"),
        None,
        OperatorEventType::TurnStarted,
    );
    orphan.payload = json!({
        "client_request_id": "req-1",
        "prompt_fingerprint": "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824"
    });
    service.append_event(orphan).unwrap();

    let error = service
        .start_turn("default", StartTurnRequest::auto("req-1", "hello"))
        .unwrap_err();
    assert!(
        error.to_string().contains("route policy"),
        "missing legacy binding must be explicit: {error}"
    );
    assert!(service.thread("default").unwrap().turns[0].prompt.is_none());
}

#[test]
fn orphaned_terminal_turn_retry_is_an_idempotency_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    let mut orphan = base_event(
        "default",
        Some("terminal-orphan-turn"),
        None,
        OperatorEventType::TurnStarted,
    );
    orphan.payload = json!({
        "client_request_id": "req-terminal",
        "prompt_fingerprint": "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
        "route_policy": StartTurnRequest::auto("req-terminal", "hello").route_policy,
    });
    service.append_event(orphan).unwrap();
    let mut terminal = base_event(
        "default",
        Some("terminal-orphan-turn"),
        None,
        OperatorEventType::TurnInterrupted,
    );
    terminal.payload = json!({"reason": "RUNTIME_RESTART"});
    service.append_event(terminal).unwrap();

    let error = service
        .start_turn("default", StartTurnRequest::auto("req-terminal", "hello"))
        .unwrap_err();
    assert!(matches!(
        error,
        TurnSubmissionError::IdempotencyConflict { .. }
    ));
}

#[test]
fn retry_with_same_client_request_and_different_prompt_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    service
        .start_turn("default", StartTurnRequest::auto("req-1", "hello"))
        .unwrap();

    let error = service
        .start_turn(
            "default",
            StartTurnRequest::auto("req-1", "different prompt"),
        )
        .unwrap_err();
    assert!(matches!(
        &error,
        TurnSubmissionError::IdempotencyConflict { .. }
    ));
    assert!(
        error.to_string().to_lowercase().contains("prompt"),
        "error should identify the retry payload mismatch: {error}"
    );
}

#[test]
fn retry_binds_the_normalized_full_route_policy() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    let original = StartTurnRequest::auto("req-1", "hello");
    service.start_turn("default", original.clone()).unwrap();
    let original_event_count = service
        .events_after("default", None, 100)
        .unwrap()
        .events
        .len();

    let mut variants = Vec::new();
    let mut request = original.clone();
    request.route_policy.mode = RouteMode::LocalOnly;
    variants.push(("mode", request));
    let mut request = original.clone();
    request.route_policy.privacy = "sovereign".into();
    variants.push(("privacy", request));
    let mut request = original.clone();
    request.route_policy.turn_budget_usd = Some(0.5);
    variants.push(("turn budget", request));
    let mut request = original.clone();
    request.route_policy.maximum_marginal_cost_usd = Some(0.1);
    variants.push(("call budget", request));
    let mut request = original.clone();
    request.route_policy.minimum_quality_class = 4;
    variants.push(("quality", request));
    let mut request = original.clone();
    request.route_policy.preferred_provider = Some("claude".into());
    variants.push(("provider", request));
    let mut request = original.clone();
    request.route_policy.preferred_model = Some("claude-sonnet".into());
    variants.push(("model", request));
    let mut request = original.clone();
    request.route_policy.allowed_models = vec!["claude-sonnet".into()];
    variants.push(("allowed models", request));
    let mut request = original.clone();
    request.route_policy.excluded_models = vec!["qwen3.5:9b".into()];
    variants.push(("excluded models", request));

    for (field, request) in variants {
        let error = service.start_turn("default", request).unwrap_err();
        assert!(matches!(
            &error,
            TurnSubmissionError::IdempotencyConflict { .. }
        ));
        assert!(
            error.to_string().contains("route policy"),
            "{field} mismatch must reject as a route-policy mismatch: {error}"
        );
        assert_eq!(
            service
                .events_after("default", None, 100)
                .unwrap()
                .events
                .len(),
            original_event_count,
            "{field} mismatch must not append"
        );
    }
}

#[test]
fn retry_accepts_semantically_equivalent_normalized_route_policy() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    let mut original = StartTurnRequest::auto("req-1", "hello");
    original.route_policy.preferred_provider = Some(" claude ".into());
    original.route_policy.preferred_model = Some(" sonnet ".into());
    original.route_policy.allowed_models = vec!["model-b".into(), "model-a".into()];
    original.route_policy.excluded_models = vec!["model-z".into(), "model-y".into()];
    original.route_policy.privacy = " STANDARD ".into();
    let first = service.start_turn("default", original).unwrap();

    let mut retry = StartTurnRequest::auto("req-1", "hello");
    retry.route_policy.preferred_provider = Some("claude".into());
    retry.route_policy.preferred_model = Some("sonnet".into());
    retry.route_policy.allowed_models = vec!["model-a".into(), "model-b".into(), "model-a".into()];
    retry.route_policy.excluded_models = vec!["model-y".into(), "model-z".into()];
    retry.route_policy.privacy = "standard".into();
    let duplicate = service.start_turn("default", retry).unwrap();

    assert!(duplicate.duplicate);
    assert_eq!(duplicate.turn_id, first.turn_id);
}

// ---------------------------------------------------------------------
// append_event validation rejections.
// ---------------------------------------------------------------------

#[test]
fn append_event_rejects_unsupported_schema_version() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    let submission = service
        .start_turn("default", StartTurnRequest::auto("req-1", "hi"))
        .unwrap();

    let mut event = base_event(
        "default",
        Some(&submission.turn_id),
        None,
        OperatorEventType::AssistantStarted,
    );
    event.schema_version = 99;

    let error = service.append_event(event).unwrap_err();
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("schema_version") || message.contains("schema version"),
        "error should name the schema version violation: {message}"
    );
}

#[test]
fn unknown_schema_events_do_not_materialize_threads_or_suppress_later_creation() {
    let dir = tempfile::tempdir().unwrap();
    let journal = OperatorJournal::new(dir.path().to_path_buf()).unwrap();
    let mut unknown = base_event("unknown-only", None, None, OperatorEventType::ThreadCreated);
    unknown.schema_version = 99;
    journal.append(&unknown).unwrap();
    let service = OperatorSessionService::new(journal);

    assert!(service.list_threads(10).unwrap().is_empty());
    let diagnostic = service.thread("unknown-only").unwrap();
    assert!(diagnostic.turns.is_empty());
    assert_eq!(diagnostic.skipped_events, 1);

    service
        .start_turn("other", StartTurnRequest::auto("other-1", "hi"))
        .unwrap();
    service
        .start_turn("unknown-only", StartTurnRequest::auto("unknown-1", "hi"))
        .unwrap();

    let summaries = service.list_threads(10).unwrap();
    assert_eq!(summaries[0].thread_id, "unknown-only");
    assert_eq!(summaries[1].thread_id, "other");
    let events = service
        .events_after("unknown-only", None, 100)
        .unwrap()
        .events;
    assert_eq!(
        events
            .iter()
            .filter(|row| {
                row.event.schema_version == OPERATOR_EVENT_SCHEMA_VERSION
                    && row.event.event_type == OperatorEventType::ThreadCreated
            })
            .count(),
        1,
        "the later valid start must create the thread despite the unknown-schema record"
    );
}

#[test]
fn rejected_unknown_turn_events_stay_diagnostic_until_valid_lifecycle_events_arrive() {
    let dir = tempfile::tempdir().unwrap();
    let journal = OperatorJournal::new(dir.path().to_path_buf()).unwrap();
    journal
        .append(&base_event(
            "replay-thread",
            Some("missing-turn"),
            None,
            OperatorEventType::UserMessage,
        ))
        .unwrap();
    journal
        .append(&base_event(
            "replay-thread",
            Some("missing-turn"),
            Some("call-1"),
            OperatorEventType::RoutePlanned,
        ))
        .unwrap();
    let service = OperatorSessionService::new(journal);

    assert!(service.list_threads(10).unwrap().is_empty());
    assert_eq!(service.thread("replay-thread").unwrap().skipped_events, 2);

    // Direct-journal lifecycle records establish the thread and its turn;
    // the earlier rejected rows remain diagnostics only.
    let journal = OperatorJournal::new(dir.path().to_path_buf()).unwrap();
    journal
        .append(&base_event(
            "replay-thread",
            None,
            None,
            OperatorEventType::ThreadCreated,
        ))
        .unwrap();
    journal
        .append(&base_event(
            "replay-thread",
            Some("valid-turn"),
            None,
            OperatorEventType::TurnStarted,
        ))
        .unwrap();
    let service = OperatorSessionService::new(journal);

    let summaries = service.list_threads(10).unwrap();
    assert_eq!(summaries.len(), 1);
    assert_eq!(summaries[0].thread_id, "replay-thread");
    let view = service.thread("replay-thread").unwrap();
    assert_eq!(view.turns.len(), 1);
    assert_eq!(view.skipped_events, 2);
}

#[test]
fn rejected_late_progress_does_not_advance_thread_recency() {
    let dir = tempfile::tempdir().unwrap();
    let journal = OperatorJournal::new(dir.path().to_path_buf()).unwrap();
    journal
        .append(&base_event(
            "closed-thread",
            None,
            None,
            OperatorEventType::ThreadCreated,
        ))
        .unwrap();
    journal
        .append(&base_event(
            "closed-thread",
            Some("closed-turn"),
            None,
            OperatorEventType::TurnStarted,
        ))
        .unwrap();
    journal
        .append(&base_event(
            "closed-thread",
            Some("closed-turn"),
            None,
            OperatorEventType::TurnCompleted,
        ))
        .unwrap();
    journal
        .append(&base_event(
            "later-thread",
            None,
            None,
            OperatorEventType::ThreadCreated,
        ))
        .unwrap();
    journal
        .append(&base_event(
            "closed-thread",
            Some("closed-turn"),
            None,
            OperatorEventType::AssistantStarted,
        ))
        .unwrap();
    let service = OperatorSessionService::new(journal);

    let summaries = service.list_threads(10).unwrap();
    assert_eq!(summaries[0].thread_id, "later-thread");
    assert_eq!(summaries[1].thread_id, "closed-thread");
    assert_eq!(service.thread("closed-thread").unwrap().skipped_events, 1);
}

#[test]
fn append_event_rejects_turn_event_missing_turn_id() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());

    let event = base_event("default", None, None, OperatorEventType::TurnCompleted);
    let error = service.append_event(event).unwrap_err();
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("turn_id"),
        "error should name the missing turn_id: {message}"
    );
}

#[test]
fn append_event_rejects_user_message_missing_turn_id() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());

    let error = service
        .append_event(base_event(
            "default",
            None,
            None,
            OperatorEventType::UserMessage,
        ))
        .unwrap_err();
    assert!(
        error.to_string().to_lowercase().contains("turn_id"),
        "error should identify the missing turn id: {error}"
    );
}

#[test]
fn append_event_rejects_nonterminal_event_for_nonexistent_turn() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());

    let event = base_event(
        "default",
        Some("missing-turn"),
        None,
        OperatorEventType::AssistantStarted,
    );
    let error = service.append_event(event).unwrap_err();
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("does not exist") || message.contains("unknown turn"),
        "error should identify the missing turn: {message}"
    );
}

#[test]
fn append_event_rejects_terminal_event_for_nonexistent_turn() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());

    let event = base_event(
        "default",
        Some("missing-turn"),
        None,
        OperatorEventType::TurnCompleted,
    );
    let error = service.append_event(event).unwrap_err();
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("does not exist") || message.contains("unknown turn"),
        "error should identify the missing turn: {message}"
    );
}

#[test]
fn append_event_allows_turn_started_to_create_synthetic_turn() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());

    service
        .append_event(base_event(
            "legacy-thread",
            Some("legacy-turn"),
            None,
            OperatorEventType::TurnStarted,
        ))
        .unwrap();

    let view = service.thread("legacy-thread").unwrap();
    assert_eq!(view.turns.len(), 1);
    assert_eq!(view.turns[0].turn_id, "legacy-turn");
}

#[test]
fn append_event_rejects_duplicate_turn_started_turn_id() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());

    service
        .append_event(base_event(
            "default",
            Some("turn-1"),
            None,
            OperatorEventType::TurnStarted,
        ))
        .unwrap();

    let error = service
        .append_event(base_event(
            "default",
            Some("turn-1"),
            None,
            OperatorEventType::TurnStarted,
        ))
        .unwrap_err();
    assert!(
        error.to_string().to_lowercase().contains("already exists"),
        "error should identify the duplicate turn id: {error}"
    );
}

#[test]
fn append_event_rejects_conflicting_turn_started_client_request_id() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());

    let mut first = base_event(
        "default",
        Some("turn-1"),
        None,
        OperatorEventType::TurnStarted,
    );
    first.payload = json!({ "client_request_id": "request-1" });
    service.append_event(first).unwrap();

    let mut conflict = base_event(
        "default",
        Some("turn-2"),
        None,
        OperatorEventType::TurnStarted,
    );
    conflict.payload = json!({ "client_request_id": "request-1" });
    let error = service.append_event(conflict).unwrap_err();
    assert!(
        error
            .to_string()
            .to_lowercase()
            .contains("client_request_id"),
        "error should identify the conflicting client request: {error}"
    );
}

#[test]
fn append_event_rejects_route_event_missing_call_id() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    let submission = service
        .start_turn("default", StartTurnRequest::auto("req-1", "hi"))
        .unwrap();

    let event = base_event(
        "default",
        Some(&submission.turn_id),
        None,
        OperatorEventType::RoutePlanned,
    );
    let error = service.append_event(event).unwrap_err();
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("call_id"),
        "error should name the missing call_id: {message}"
    );
}

#[test]
fn append_event_rejects_tool_event_missing_call_id() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    let submission = service
        .start_turn("default", StartTurnRequest::auto("req-1", "hi"))
        .unwrap();

    let event = base_event(
        "default",
        Some(&submission.turn_id),
        None,
        OperatorEventType::ToolCallStarted,
    );
    let error = service.append_event(event).unwrap_err();
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("call_id"),
        "error should name the missing call_id: {message}"
    );
}

#[test]
fn append_event_accepts_well_formed_events_before_terminal() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    let submission = service
        .start_turn("default", StartTurnRequest::auto("req-1", "hi"))
        .unwrap();

    let route = base_event(
        "default",
        Some(&submission.turn_id),
        Some("call-1"),
        OperatorEventType::RoutePlanned,
    );
    service.append_event(route).unwrap();

    let tool = base_event(
        "default",
        Some(&submission.turn_id),
        Some("call-1"),
        OperatorEventType::ToolCallStarted,
    );
    service.append_event(tool).unwrap();
}

#[test]
fn append_event_rejects_nonterminal_event_on_terminal_turn() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    let submission = service
        .start_turn("default", StartTurnRequest::auto("req-1", "hi"))
        .unwrap();

    let completed = base_event(
        "default",
        Some(&submission.turn_id),
        None,
        OperatorEventType::TurnCompleted,
    );
    service.append_event(completed).unwrap();

    let assistant = base_event(
        "default",
        Some(&submission.turn_id),
        None,
        OperatorEventType::AssistantStarted,
    );
    let error = service.append_event(assistant).unwrap_err();
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("terminal"),
        "error should name the terminal-state violation: {message}"
    );
}

#[test]
fn append_event_rejects_cancel_request_on_terminal_turn() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    let submission = service
        .start_turn("default", StartTurnRequest::auto("req-1", "hi"))
        .unwrap();

    let completed = base_event(
        "default",
        Some(&submission.turn_id),
        None,
        OperatorEventType::TurnCompleted,
    );
    service.append_event(completed).unwrap();

    let cancel = base_event(
        "default",
        Some(&submission.turn_id),
        None,
        OperatorEventType::TurnCancelRequested,
    );
    let error = service.append_event(cancel).unwrap_err();
    let message = error.to_string().to_lowercase();
    assert!(
        message.contains("terminal"),
        "error should name the terminal-state violation: {message}"
    );
}

#[test]
fn append_event_rejects_second_terminal_event() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    let submission = service
        .start_turn("default", StartTurnRequest::auto("req-1", "hi"))
        .unwrap();
    service
        .append_event(base_event(
            "default",
            Some(&submission.turn_id),
            None,
            OperatorEventType::TurnCompleted,
        ))
        .unwrap();

    let error = service
        .append_event(base_event(
            "default",
            Some(&submission.turn_id),
            None,
            OperatorEventType::TurnInterrupted,
        ))
        .unwrap_err();
    assert!(
        error.to_string().to_lowercase().contains("terminal"),
        "error should reject every event after terminal state: {error}"
    );
}

#[test]
fn operator_cancelled_interruption_requires_prior_cancel_request() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    let submission = service
        .start_turn("default", StartTurnRequest::auto("req-1", "hi"))
        .unwrap();
    let mut interrupted = base_event(
        "default",
        Some(&submission.turn_id),
        None,
        OperatorEventType::TurnInterrupted,
    );
    interrupted.payload = json!({ "reason": "OPERATOR_CANCELLED" });

    let error = service.append_event(interrupted).unwrap_err();
    assert!(
        error.to_string().to_lowercase().contains("cancel"),
        "error should require prior cancellation intent: {error}"
    );
}

#[test]
fn pending_cancellation_rejects_completion_and_closes_as_operator_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    let submission = service
        .start_turn("default", StartTurnRequest::auto("req-1", "hi"))
        .unwrap();
    service
        .append_event(base_event(
            "default",
            Some(&submission.turn_id),
            None,
            OperatorEventType::TurnCancelRequested,
        ))
        .unwrap();

    let completion = base_event(
        "default",
        Some(&submission.turn_id),
        None,
        OperatorEventType::TurnCompleted,
    );
    assert!(service.append_event(completion).is_err());

    let mut interrupted = base_event(
        "default",
        Some(&submission.turn_id),
        None,
        OperatorEventType::TurnInterrupted,
    );
    interrupted.payload = json!({ "reason": "OPERATOR_CANCELLED" });
    service.append_event(interrupted).unwrap();
    assert_eq!(
        service.thread("default").unwrap().turns[0].status,
        "interrupted"
    );
}

#[test]
fn restart_recovery_closes_pending_cancellation_as_operator_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    let submission = service
        .start_turn("default", StartTurnRequest::auto("req-1", "hi"))
        .unwrap();
    service
        .append_event(base_event(
            "default",
            Some(&submission.turn_id),
            None,
            OperatorEventType::TurnCancelRequested,
        ))
        .unwrap();

    assert_eq!(service.recover_interrupted().unwrap(), 1);
    let events = service.events_after("default", None, 100).unwrap().events;
    let recovered = events.last().unwrap();
    assert_eq!(
        recovered.event.event_type,
        OperatorEventType::TurnInterrupted
    );
    assert_eq!(recovered.event.payload["reason"], "OPERATOR_CANCELLED");
}

// ---------------------------------------------------------------------
// events_after: thread filtering across interleaved threads, cursor
// advance over nonmatching rows.
// ---------------------------------------------------------------------

#[test]
fn events_after_filters_thread_and_advances_cursor_across_interleaved_threads() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());

    service
        .start_turn("thread-a", StartTurnRequest::auto("req-a1", "hi a1"))
        .unwrap();
    service
        .start_turn("thread-b", StartTurnRequest::auto("req-b1", "hi b1"))
        .unwrap();
    service
        .start_turn("thread-a", StartTurnRequest::auto("req-a2", "hi a2"))
        .unwrap();
    service
        .start_turn("thread-b", StartTurnRequest::auto("req-b2", "hi b2"))
        .unwrap();
    service
        .start_turn("thread-a", StartTurnRequest::auto("req-a3", "hi a3"))
        .unwrap();

    // Page through thread-a's events with a small limit. Even though
    // thread-b's events are interleaved in the global stream, we must never
    // reread a row and never miss one.
    let mut cursor: Option<String> = None;
    let mut collected_ids = Vec::new();
    loop {
        let page = service
            .events_after("thread-a", cursor.as_deref(), 2)
            .unwrap();
        if page.events.is_empty() {
            assert_eq!(page.next_cursor.as_deref(), cursor.as_deref());
            break;
        }
        for row in &page.events {
            assert_eq!(row.event.thread_id, "thread-a");
            collected_ids.push(row.event.event_id.clone());
        }
        cursor = page.next_cursor.clone();
    }

    // thread_created + 3 turns * (turn_started + user_message) = 7.
    assert_eq!(collected_ids.len(), 7);
    let unique: std::collections::HashSet<_> = collected_ids.iter().collect();
    assert_eq!(unique.len(), 7, "no row was replayed twice");
}

#[test]
fn events_after_advances_cursor_past_trailing_nonmatching_events() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());

    service
        .start_turn("thread-a", StartTurnRequest::auto("req-a1", "hi a1"))
        .unwrap();
    service
        .start_turn("thread-b", StartTurnRequest::auto("req-b1", "hi b1"))
        .unwrap();
    service
        .start_turn("thread-a", StartTurnRequest::auto("req-a2", "hi a2"))
        .unwrap();
    // thread-b trails last in the global stream.
    service
        .start_turn("thread-b", StartTurnRequest::auto("req-b2", "hi b2"))
        .unwrap();

    let page = service.events_after("thread-a", None, 100).unwrap();
    assert_eq!(page.events.len(), 5, "thread_created + 2 turns * 2 events");
    assert!(page
        .events
        .iter()
        .all(|row| row.event.thread_id == "thread-a"));

    // Polling again from next_cursor finds nothing new: the cursor advanced
    // all the way past the trailing thread-b events instead of getting
    // stuck at thread-a's last actual match.
    let empty = service
        .events_after("thread-a", page.next_cursor.as_deref(), 100)
        .unwrap();
    assert_eq!(empty.events.len(), 0);
    assert_eq!(empty.next_cursor, page.next_cursor);

    // A fresh thread-a event appended afterward is still picked up from
    // that same cursor: we did not lose our place either.
    service
        .start_turn("thread-a", StartTurnRequest::auto("req-a3", "hi a3"))
        .unwrap();
    let more = service
        .events_after("thread-a", page.next_cursor.as_deref(), 100)
        .unwrap();
    assert_eq!(
        more.events.len(),
        2,
        "turn_started + user_message for the new turn"
    );
    assert!(more
        .events
        .iter()
        .all(|row| row.event.thread_id == "thread-a"));
}

// ---------------------------------------------------------------------
// list_threads: bounded output.
// ---------------------------------------------------------------------

#[test]
fn list_threads_bounds_output_and_orders_most_recent_first() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());

    service
        .start_turn("thread-1", StartTurnRequest::auto("r1", "hi"))
        .unwrap();
    service
        .start_turn("thread-2", StartTurnRequest::auto("r2", "hi"))
        .unwrap();
    service
        .start_turn("thread-3", StartTurnRequest::auto("r3", "hi"))
        .unwrap();

    let all = service.list_threads(10).unwrap();
    assert_eq!(all.len(), 3);

    let bounded = service.list_threads(2).unwrap();
    assert_eq!(bounded.len(), 2, "limit bounds the returned thread count");
    assert_eq!(bounded[0].thread_id, "thread-3");
    assert_eq!(bounded[1].thread_id, "thread-2");
}

// ---------------------------------------------------------------------
// thread(): journal-level damage surfaced distinctly from event-level
// rejects.
// ---------------------------------------------------------------------

#[test]
fn thread_view_surfaces_journal_damage_as_skipped_lines() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());
    service
        .start_turn("default", StartTurnRequest::auto("req-1", "hello"))
        .unwrap();

    // Journal-level damage: a complete but unparseable line appended
    // directly to the stream file, as a crashed or foreign writer might
    // leave behind. This is below the event contract entirely — no
    // event_id, no thread_id — so it must surface as `skipped_lines`
    // (journal damage), never as `skipped_events` (schema/state rejects).
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(dir.path().join("operator_events.jsonl"))
        .unwrap();
    writeln!(file, "this is not a journal envelope").unwrap();
    drop(file);

    let view = service.thread("default").unwrap();
    assert_eq!(
        view.skipped_lines, 1,
        "journal damage is surfaced, counted once"
    );
    assert_eq!(
        view.skipped_events, 0,
        "no schema/state-level rejects occurred"
    );
    assert_eq!(view.turns.len(), 1, "valid events still project");
    assert_eq!(view.turns[0].prompt.as_deref(), Some("hello"));
}

// ---------------------------------------------------------------------
// thread(): materialized turn status transitions.
// ---------------------------------------------------------------------

#[test]
fn thread_view_reflects_turn_completed_transition() {
    let dir = tempfile::tempdir().unwrap();
    let service = test_service(dir.path());

    let submission = service
        .start_turn("default", StartTurnRequest::auto("req-1", "hello"))
        .unwrap();

    let before = service.thread("default").unwrap();
    assert_eq!(before.turns.len(), 1);
    assert_eq!(before.turns[0].status, "open");
    assert_eq!(before.turns[0].turn_id, submission.turn_id);

    let completed = base_event(
        "default",
        Some(&submission.turn_id),
        None,
        OperatorEventType::TurnCompleted,
    );
    service.append_event(completed).unwrap();

    let after = service.thread("default").unwrap();
    assert_eq!(after.turns.len(), 1);
    assert_eq!(after.turns[0].status, "completed");
}
