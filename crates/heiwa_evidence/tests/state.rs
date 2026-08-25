//! Materialized state: replay folds, restart recovery, compaction.

use heiwa_evidence::{
    compact_stream, read_stream, recover_interrupted, EvidenceTransport, JsonlTransport,
    PersistedWorkerLease, PersistedWorkerSession, WorkerStateView,
};

fn session(id: &str, status: &str) -> PersistedWorkerSession {
    PersistedWorkerSession {
        session_id: id.to_string(),
        node_id: "node-1".to_string(),
        instance_id: "inst-1".to_string(),
        runtime: "claude".to_string(),
        runtime_version: "1.0".to_string(),
        worker_version: "0.1".to_string(),
        protocol: "v1".to_string(),
        capabilities_json: "[\"exec\"]".to_string(),
        metadata_json: "{}".to_string(),
        max_concurrency: 1,
        active_tasks: 0,
        status: status.to_string(),
        load: 0.0,
        created_at: "2026-07-16T00:00:00Z".to_string(),
        updated_at: "2026-07-16T00:00:00Z".to_string(),
        expires_at: "2026-07-16T01:00:00Z".to_string(),
        last_seen_at: "2026-07-16T00:00:00Z".to_string(),
        closed_at: None,
        current_task_id: None,
        lease_id: None,
    }
}

fn lease(id: &str, session_id: &str, status: &str) -> PersistedWorkerLease {
    PersistedWorkerLease {
        lease_id: id.to_string(),
        task_id: format!("task-{id}"),
        session_id: session_id.to_string(),
        node_id: "node-1".to_string(),
        capability: "exec".to_string(),
        status: status.to_string(),
        issued_at: "2026-07-16T00:00:00Z".to_string(),
        updated_at: "2026-07-16T00:00:00Z".to_string(),
        expires_at: "2026-07-16T01:00:00Z".to_string(),
        acked_at: None,
        completed_at: None,
        failure_code: None,
        reason: None,
    }
}

#[test]
fn concurrent_acquisition_of_one_capability_has_exactly_one_winner() {
    let dir = tempfile::tempdir().expect("tempdir");
    let first = JsonlTransport::new(dir.path().to_path_buf()).expect("first transport");
    let second = JsonlTransport::new(dir.path().to_path_buf()).expect("second transport");
    let start = std::sync::Arc::new(std::sync::Barrier::new(3));

    let first_start = std::sync::Arc::clone(&start);
    let first_worker = std::thread::spawn(move || {
        first_start.wait();
        first
            .try_acquire_worker_lease(lease("first", "work-first", "issued"))
            .expect("first acquisition")
    });

    let second_start = std::sync::Arc::clone(&start);
    let second_worker = std::thread::spawn(move || {
        second_start.wait();
        second
            .try_acquire_worker_lease(lease("second", "work-second", "issued"))
            .expect("second acquisition")
    });

    start.wait();
    let outcomes = [
        first_worker.join().expect("first worker"),
        second_worker.join().expect("second worker"),
    ];

    assert_eq!(
        outcomes.iter().filter(|outcome| outcome.is_none()).count(),
        1,
        "one caller acquires; the other receives the durable holder"
    );
    let held = outcomes
        .iter()
        .find_map(|outcome| outcome.as_ref())
        .expect("one caller is refused");
    assert_eq!(held.capability, "exec");

    let view = WorkerStateView::replay(dir.path()).expect("replay");
    let live = view
        .leases
        .values()
        .filter(|lease| matches!(lease.status.as_str(), "issued" | "acked"))
        .count();
    assert_eq!(live, 1, "the journal must contain one live owner");
}

#[test]
fn a_damaged_lease_stream_fails_closed_instead_of_minting_a_second_owner() {
    let dir = tempfile::tempdir().expect("tempdir");
    let stream = dir.path().join("worker_leases.jsonl");
    std::fs::write(&stream, b"{not valid json}\n").expect("damage fixture");
    let transport = JsonlTransport::new(dir.path().to_path_buf()).expect("transport");

    let error = transport
        .try_acquire_worker_lease(lease("candidate", "work-candidate", "issued"))
        .expect_err("unknown lease state cannot be treated as free");

    assert!(error.to_string().contains("damaged"), "{error}");
    assert_eq!(
        std::fs::read(&stream).expect("stream"),
        b"{not valid json}\n",
        "a failed-closed acquisition must append nothing"
    );
}

#[test]
fn an_expired_capability_is_closed_before_its_successor_is_acquired() {
    let dir = tempfile::tempdir().expect("tempdir");
    let transport = JsonlTransport::new(dir.path().to_path_buf()).expect("transport");
    let mut expired = lease("expired", "work-old", "issued");
    expired.expires_at = "2026-07-16T01:00:00Z".to_string();
    transport
        .upsert_worker_lease(expired)
        .expect("seed expired lease");

    let mut successor = lease("successor", "work-new", "issued");
    successor.issued_at = "2026-07-16T02:00:00Z".to_string();
    successor.updated_at = successor.issued_at.clone();
    successor.expires_at = "2026-07-16T03:00:00Z".to_string();
    assert!(
        transport
            .try_acquire_worker_lease(successor)
            .expect("acquire after expiry")
            .is_none(),
        "an expired holder no longer owns the capability"
    );

    let view = WorkerStateView::replay(dir.path()).expect("replay");
    assert_eq!(view.leases["expired"].status, "expired");
    assert_eq!(view.leases["successor"].status, "issued");
}

#[test]
fn worker_state_view_folds_last_record_per_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    let transport = JsonlTransport::new(dir.path().to_path_buf()).expect("transport");

    transport
        .upsert_worker_session(session("s1", "active"))
        .unwrap();
    transport
        .upsert_worker_session(session("s2", "active"))
        .unwrap();
    transport
        .upsert_worker_session(session("s1", "busy"))
        .unwrap();
    transport
        .upsert_worker_lease(lease("l1", "s1", "issued"))
        .unwrap();
    transport
        .upsert_worker_lease(lease("l1", "s1", "completed"))
        .unwrap();

    let view = WorkerStateView::replay(dir.path()).expect("replay");
    assert_eq!(view.sessions.len(), 2);
    assert_eq!(view.sessions["s1"].status, "busy", "last write wins");
    assert_eq!(view.sessions["s2"].status, "active");
    assert_eq!(view.leases.len(), 1);
    assert_eq!(view.leases["l1"].status, "completed");
}

#[test]
fn worker_state_view_applies_session_close_tombstones() {
    let dir = tempfile::tempdir().expect("tempdir");
    let transport = JsonlTransport::new(dir.path().to_path_buf()).expect("transport");

    transport
        .upsert_worker_session(session("s1", "active"))
        .unwrap();
    transport.close_session("s1".to_string()).unwrap();

    let view = WorkerStateView::replay(dir.path()).expect("replay");
    assert_eq!(view.sessions["s1"].status, "closed");
}

#[test]
fn recover_interrupted_closes_live_sessions_and_revokes_open_leases() {
    let dir = tempfile::tempdir().expect("tempdir");
    let transport = JsonlTransport::new(dir.path().to_path_buf()).expect("transport");

    transport
        .upsert_worker_session(session("live", "active"))
        .unwrap();
    transport
        .upsert_worker_session(session("done", "closed"))
        .unwrap();
    transport
        .upsert_worker_lease(lease("open", "live", "acked"))
        .unwrap();
    transport
        .upsert_worker_lease(lease("finished", "live", "completed"))
        .unwrap();

    let report = recover_interrupted(dir.path(), &transport).expect("recover");
    assert_eq!(report.sessions_closed, 1);
    assert_eq!(report.leases_revoked, 1);

    let view = WorkerStateView::replay(dir.path()).expect("replay after recovery");
    assert_eq!(view.sessions["live"].status, "closed");
    assert!(view.sessions["live"].closed_at.is_some());
    assert_eq!(
        view.sessions["done"].status, "closed",
        "already closed untouched"
    );
    assert_eq!(view.leases["open"].status, "revoked");
    assert_eq!(
        view.leases["open"].failure_code.as_deref(),
        Some("RUNTIME_RESTART")
    );
    assert_eq!(view.leases["finished"].status, "completed");

    // Recovery is idempotent: a second pass finds nothing live.
    let second = recover_interrupted(dir.path(), &transport).expect("recover again");
    assert_eq!(second.sessions_closed, 0);
    assert_eq!(second.leases_revoked, 0);
}

#[test]
fn compact_stream_keeps_only_last_record_per_key() {
    let dir = tempfile::tempdir().expect("tempdir");
    let transport = JsonlTransport::new(dir.path().to_path_buf()).expect("transport");

    transport
        .upsert_worker_session(session("s1", "active"))
        .unwrap();
    transport
        .upsert_worker_session(session("s1", "busy"))
        .unwrap();
    transport
        .upsert_worker_session(session("s1", "closed"))
        .unwrap();
    transport
        .upsert_worker_session(session("s2", "active"))
        .unwrap();

    let report = compact_stream(dir.path(), "worker_sessions", "session_id").expect("compact");
    assert_eq!(report.records_before, 4);
    assert_eq!(report.records_after, 2);

    let stream = read_stream(dir.path(), "worker_sessions").expect("replay");
    assert_eq!(stream.events.len(), 2);
    let view = WorkerStateView::replay(dir.path()).expect("view");
    assert_eq!(view.sessions["s1"].status, "closed");
    assert_eq!(view.sessions["s2"].status, "active");
}

#[test]
fn evidence_runtime_records_drex_decision_with_route_link() {
    use heiwa_evidence::{EvidenceRuntime, PersistedDrexDecision};

    let dir = tempfile::tempdir().expect("tempdir");
    let transport = JsonlTransport::new(dir.path().to_path_buf()).expect("transport");
    let runtime = EvidenceRuntime::new(transport);

    let decision = PersistedDrexDecision {
        drex_decision_id: "drex:req-1:1".to_string(),
        request_id: "req-1".to_string(),
        task_id: "task-1".to_string(),
        active_tier: "meso".to_string(),
        route_runtime: "ollama".to_string(),
        route_model: "qwen3.5:9b".to_string(),
        scope: 0.1,
        abstraction: 0.2,
        context_span: 0.3,
        execution_proximity: 0.4,
        blast_radius: 0.5,
        coordination_load: 0.6,
        latency_pressure: 0.7,
        macro_score: 0.1,
        meso_score: 0.9,
        micro_score: 0.2,
        score_confidence: 0.8,
        authority_required: "none".to_string(),
        requires_approval: false,
        reasons_json: "[]".to_string(),
        vector_json: "{}".to_string(),
        scorecard_json: "{}".to_string(),
        gate_json: "{}".to_string(),
        policy_version: "v1".to_string(),
        created_at_ms: 1,
    };

    let rt = tokio_free_block_on(runtime.record_drex_decision(decision.clone()));
    let recorded = rt.expect("record decision");
    assert_eq!(recorded, decision);

    let decisions = read_stream(dir.path(), "drex_decisions").expect("decisions");
    assert_eq!(decisions.events.len(), 1);
    let links = read_stream(dir.path(), "route_links").expect("links");
    assert_eq!(links.events.len(), 1);
    assert_eq!(
        links.events[0].record["drex_decision_id"].as_str(),
        Some("drex:req-1:1")
    );
}

/// The runtime methods are async only for call-site stability; they never
/// await anything, so polling once on a no-op waker drives them to completion
/// without pulling a tokio dev-dependency into this crate.
fn tokio_free_block_on<F: std::future::Future>(future: F) -> F::Output {
    use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};

    fn noop_raw_waker() -> RawWaker {
        fn clone(_: *const ()) -> RawWaker {
            noop_raw_waker()
        }
        fn noop(_: *const ()) {}
        RawWaker::new(
            std::ptr::null(),
            &RawWakerVTable::new(clone, noop, noop, noop),
        )
    }

    let waker = unsafe { Waker::from_raw(noop_raw_waker()) };
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    match future.as_mut().poll(&mut context) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("evidence runtime futures must complete synchronously"),
    }
}
