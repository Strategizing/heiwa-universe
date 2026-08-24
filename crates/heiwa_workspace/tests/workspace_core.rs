//! One Work taking a real hold on a real repository.
//!
//! The unit tests prove each piece. This proves the pieces compose: the events
//! survive the operator writer, the lease survives replay, and the whole thing
//! folds back into the A1-a `Work` aggregate.

use heiwa_evidence::{JsonlTransport, OperatorJournal};
use heiwa_session::operator::OperatorSessionService;
use heiwa_work::{fold, work_created_event, WorkId};
use heiwa_workspace::{
    acquire_writer_lease, create_worktree_in, diff_projection_in, git, release_writer_lease,
    snapshot_in, workspace_prepared_event, workspace_released_event,
};

fn repo(dir: &std::path::Path) {
    git(dir, &["init", "-q", "-b", "main", "."]).expect("init");
    git(dir, &["config", "user.email", "test@heiwa.ltd"]).expect("email");
    git(dir, &["config", "user.name", "test"]).expect("name");
    std::fs::write(dir.join("a.txt"), "one\n").expect("write");
    git(dir, &["add", "."]).expect("add");
    git(dir, &["commit", "-qm", "one"]).expect("commit");
}

#[test]
fn preparing_a_workspace_replays_onto_the_work_that_owns_it() {
    let source = tempfile::tempdir().expect("source");
    repo(source.path());
    let holding = tempfile::tempdir().expect("holding");
    let evidence = tempfile::tempdir().expect("evidence");

    let service =
        OperatorSessionService::new(OperatorJournal::new(evidence.path().to_path_buf()).unwrap());
    service.ensure_thread("thread-1").expect("thread");

    let work_id = WorkId::generate(|| "abc".to_string());
    service
        .append_event(work_created_event(
            &work_id,
            "thread-1",
            "prepare the release",
            "install-1",
            "2026-08-24T00:00:00Z",
            || "evt-1".to_string(),
        ))
        .expect("work_created");

    let handle =
        create_worktree_in(source.path(), holding.path(), work_id.as_str()).expect("worktree");
    let snapshot = snapshot_in(source.path()).expect("snapshot");

    service
        .append_event(workspace_prepared_event(
            work_id.as_str(),
            "thread-1",
            &snapshot.root,
            &handle,
            "2026-08-24T00:01:00Z",
            || "evt-2".to_string(),
        ))
        .expect("workspace_prepared");

    let page = service.events_after("thread-1", None, 64).expect("replay");
    let events: Vec<_> = page.events.into_iter().map(|row| row.event).collect();
    let projection = fold(&events);
    let work = projection.work(work_id.as_str()).expect("work replays");

    assert_eq!(
        work.revision, 1,
        "workspace events do not advance the Work revision yet"
    );
    assert_eq!(
        projection.skipped_events, 0,
        "a scoped workspace event is not damage"
    );
}

#[test]
fn a_workspace_event_without_its_work_is_refused_by_the_writer() {
    let source = tempfile::tempdir().expect("source");
    repo(source.path());
    let holding = tempfile::tempdir().expect("holding");
    let evidence = tempfile::tempdir().expect("evidence");

    let service =
        OperatorSessionService::new(OperatorJournal::new(evidence.path().to_path_buf()).unwrap());
    service.ensure_thread("thread-1").expect("thread");

    let handle = create_worktree_in(source.path(), holding.path(), "work-abc").expect("worktree");
    let mut event = workspace_prepared_event(
        "work-abc",
        "thread-1",
        "/repo",
        &handle,
        "2026-08-24T00:01:00Z",
        || "evt-1".to_string(),
    );
    event.work_id = None;

    let error = service
        .append_event(event)
        .expect_err("an unscoped workspace event must not reach the journal");
    assert!(error.to_string().contains("requires work_id"), "{error}");
}

#[test]
fn the_whole_hold_can_be_taken_and_given_back() {
    let source = tempfile::tempdir().expect("source");
    repo(source.path());
    let holding = tempfile::tempdir().expect("holding");
    let evidence = tempfile::tempdir().expect("evidence");
    let transport = JsonlTransport::new(evidence.path().to_path_buf()).expect("transport");

    let snapshot = snapshot_in(source.path()).expect("snapshot");
    let lease = acquire_writer_lease(
        evidence.path(),
        &transport,
        "work-abc",
        &snapshot.root,
        "install-1",
        "2026-08-24T00:00:00Z",
        "2026-08-24T01:00:00Z",
        || "lease-1".to_string(),
    )
    .expect("acquire");

    let handle = create_worktree_in(source.path(), holding.path(), "work-abc").expect("worktree");
    std::fs::write(
        std::path::Path::new(&handle.path).join("a.txt"),
        "one\ntwo\n",
    )
    .expect("edit inside the worktree");

    let diff = diff_projection_in(std::path::Path::new(&handle.path), 50).expect("diff");
    assert_eq!(diff.total_files, 1);
    assert_eq!(diff.files[0].path, "a.txt");

    // The user's own working tree never moved.
    let untouched = std::fs::read_to_string(source.path().join("a.txt")).expect("read");
    assert_eq!(untouched, "one\n");

    release_writer_lease(&transport, &lease, "2026-08-24T00:30:00Z").expect("release");
    acquire_writer_lease(
        evidence.path(),
        &transport,
        "work-def",
        &snapshot.root,
        "install-1",
        "2026-08-24T00:31:00Z",
        "2026-08-24T01:31:00Z",
        || "lease-2".to_string(),
    )
    .expect("the repository is free again");

    let _ = workspace_released_event(
        "work-abc",
        "thread-1",
        &snapshot.root,
        "2026-08-24T00:30:00Z",
        || "evt-3".to_string(),
    );
}
