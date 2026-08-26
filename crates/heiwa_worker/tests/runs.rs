use heiwa_worker::{
    fold_runs, pane_closed_event, pane_opened_event, worker_exited_event, worker_heartbeat_event,
    worker_launched_event, PaneIdentity, PaneState, WorkerIdentity, WorkerState, SCHEMA_VERSION,
};

fn identity(work: &str, worker: &str) -> WorkerIdentity {
    WorkerIdentity {
        schema_version: SCHEMA_VERSION,
        worker_id: worker.into(),
        work_id: work.into(),
        thread_id: "thread-1".into(),
        provider: "claude".into(),
        provider_session_ref: None,
        executable_path: "/usr/local/bin/claude".into(),
        executable_sha256: "a".repeat(64),
        cwd: "/tmp/worktrees/w".into(),
        repo_root: "/tmp/repo".into(),
        branch: "heiwa/w".into(),
        base_commit: "b".repeat(40),
        lease_id: "lease-1".into(),
        installation_id: "install-1".into(),
        started_at: "2026-08-26T00:00:00Z".into(),
    }
}

fn pane(work: &str, worker: &str, pane_id: &str) -> PaneIdentity {
    PaneIdentity {
        schema_version: SCHEMA_VERSION,
        pane_id: pane_id.into(),
        work_id: work.into(),
        worker_id: worker.into(),
        cwd: "/tmp/worktrees/w".into(),
        repo_root: "/tmp/repo".into(),
        branch: "heiwa/w".into(),
        opened_at: "2026-08-26T00:00:00Z".into(),
    }
}

fn ids() -> impl FnMut() -> String {
    let mut n = 0;
    move || {
        n += 1;
        format!("e{n}")
    }
}

#[test]
fn a_launched_worker_that_has_not_reported_is_starting() {
    let mut next = ids();
    let events = vec![worker_launched_event(
        &identity("work-1", "worker-1"),
        "2026-08-26T00:00:00Z",
        &mut next,
    )];
    let runs = fold_runs(&events, "work-1");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].worker_state, WorkerState::Starting);
    assert_eq!(runs[0].exit_code, None);
}

#[test]
fn a_heartbeat_promotes_starting_to_live() {
    let mut next = ids();
    let id = identity("work-1", "worker-1");
    let events = vec![
        worker_launched_event(&id, "2026-08-26T00:00:00Z", &mut next),
        worker_heartbeat_event(&id, 4242, "2026-08-26T00:00:01Z", &mut next),
    ];
    let runs = fold_runs(&events, "work-1");
    assert_eq!(runs[0].worker_state, WorkerState::Live);
}

#[test]
fn a_clean_exit_is_exited_and_a_failure_code_is_failed() {
    let id = identity("work-1", "worker-1");

    let mut next = ids();
    let clean = vec![
        worker_launched_event(&id, "2026-08-26T00:00:00Z", &mut next),
        worker_exited_event(&id, Some(0), None, "2026-08-26T00:00:01Z", &mut next),
    ];
    assert_eq!(
        fold_runs(&clean, "work-1")[0].worker_state,
        WorkerState::Exited
    );

    let mut next = ids();
    let broken = vec![
        worker_launched_event(&id, "2026-08-26T00:00:00Z", &mut next),
        worker_exited_event(
            &id,
            None,
            Some("spawn_failed".into()),
            "2026-08-26T00:00:01Z",
            &mut next,
        ),
    ];
    assert_eq!(
        fold_runs(&broken, "work-1")[0].worker_state,
        WorkerState::Failed
    );
}

#[test]
fn a_pane_bound_to_a_worker_that_never_went_live_is_unverified() {
    let mut next = ids();
    let id = identity("work-1", "worker-1");
    let events = vec![
        worker_launched_event(&id, "2026-08-26T00:00:00Z", &mut next),
        pane_opened_event(
            &pane("work-1", "worker-1", "pane-1"),
            "thread-1",
            "2026-08-26T00:00:00Z",
            &mut next,
        ),
    ];
    let runs = fold_runs(&events, "work-1");
    assert_eq!(runs[0].pane_id.as_deref(), Some("pane-1"));
    assert_eq!(runs[0].pane_state, Some(PaneState::Unverified));
}

#[test]
fn a_pane_for_an_unknown_worker_is_not_promoted_to_a_run() {
    let mut next = ids();
    let events = vec![pane_opened_event(
        &pane("work-1", "ghost", "pane-1"),
        "thread-1",
        "2026-08-26T00:00:00Z",
        &mut next,
    )];
    // The spec forbids treating a pane as a verified worker merely because it
    // exists. A pane with no launched worker produces no run row.
    assert!(fold_runs(&events, "work-1").is_empty());
}

#[test]
fn runs_from_another_work_are_excluded() {
    let mut next = ids();
    let events = vec![
        worker_launched_event(
            &identity("work-1", "worker-1"),
            "2026-08-26T00:00:00Z",
            &mut next,
        ),
        worker_launched_event(
            &identity("work-2", "worker-2"),
            "2026-08-26T00:00:01Z",
            &mut next,
        ),
    ];
    let runs = fold_runs(&events, "work-1");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].worker_id, "worker-1");
}

#[test]
fn a_closed_pane_reports_its_tail_and_dropped_count() {
    let mut next = ids();
    let id = identity("work-1", "worker-1");
    let p = pane("work-1", "worker-1", "pane-1");
    let events = vec![
        worker_launched_event(&id, "2026-08-26T00:00:00Z", &mut next),
        pane_opened_event(&p, "thread-1", "2026-08-26T00:00:00Z", &mut next),
        worker_exited_event(&id, Some(0), None, "2026-08-26T00:00:02Z", &mut next),
        pane_closed_event(
            &p,
            "thread-1",
            vec!["last".into()],
            7,
            "2026-08-26T00:00:03Z",
            &mut next,
        ),
    ];
    let runs = fold_runs(&events, "work-1");
    assert_eq!(runs[0].pane_tail, vec!["last".to_string()]);
    assert_eq!(runs[0].pane_dropped_lines, 7);
    assert_eq!(runs[0].pane_state, Some(PaneState::Done));
}

#[test]
fn an_event_whose_envelope_names_a_work_is_not_second_guessed_from_payload() {
    let mut next = ids();
    let mut event = worker_launched_event(
        &identity("work-1", "worker-1"),
        "2026-08-26T00:00:00Z",
        &mut next,
    );
    event.payload = serde_json::json!({ "worker_id": "worker-1" });
    // A payload that no longer deserializes fully must not silently vanish; the
    // envelope still says this Work has a worker.
    let runs = fold_runs(&[event], "work-1");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].worker_id, "worker-1");
    assert_eq!(runs[0].provider, None);
}
