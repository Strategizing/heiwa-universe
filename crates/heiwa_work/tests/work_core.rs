//! Work folded from events that actually went through the operator journal.
//!
//! The unit tests fold in-memory slices. This one proves the same events
//! survive the append path, its validation, and replay.

use heiwa_evidence::OperatorJournal;
use heiwa_session::operator::OperatorSessionService;
use heiwa_work::{fold, work_created_event, work_linked_event, WorkId, WorkLinkOrigin};

fn service(root: &std::path::Path) -> OperatorSessionService {
    OperatorSessionService::new(OperatorJournal::new(root.to_path_buf()).expect("journal"))
}

#[test]
fn work_survives_the_append_path_and_replays_identically() {
    let dir = tempfile::tempdir().expect("tempdir");
    let service = service(dir.path());
    service.ensure_thread("thread-1").expect("thread");

    let work_id = WorkId::generate(|| "abc".to_string());
    service
        .append_event(work_created_event(
            &work_id,
            "thread-1",
            "prepare the release",
            "installation-1",
            "2026-08-22T00:00:00Z",
            || "evt-1".to_string(),
        ))
        .expect("append work_created");

    let page = service
        .events_after("thread-1", None, 64)
        .expect("replay thread");
    let events: Vec<_> = page.events.into_iter().map(|row| row.event).collect();

    let projection = fold(&events);
    let work = projection.work(work_id.as_str()).expect("work replays");
    assert_eq!(work.intent, "prepare the release");
    assert_eq!(work.revision, 1);
    assert!(!work.is_replicable(), "local work has no node identity yet");
}

#[test]
fn a_work_event_missing_its_scope_is_refused_by_the_writer() {
    let dir = tempfile::tempdir().expect("tempdir");
    let service = service(dir.path());
    service.ensure_thread("thread-1").expect("thread");

    let work_id = WorkId::generate(|| "abc".to_string());
    let mut event = work_created_event(
        &work_id,
        "thread-1",
        "prepare the release",
        "installation-1",
        "2026-08-22T00:00:00Z",
        || "evt-1".to_string(),
    );
    event.work_id = None;

    let error = service
        .append_event(event)
        .expect_err("an unscoped work event must not reach the journal");
    assert!(error.to_string().contains("requires work_id"), "{error}");
}

#[test]
fn a_linked_thread_replays_onto_the_same_work() {
    let dir = tempfile::tempdir().expect("tempdir");
    let service = service(dir.path());
    service.ensure_thread("thread-1").expect("thread");

    let work_id = WorkId::generate(|| "abc".to_string());
    service
        .append_event(work_created_event(
            &work_id,
            "thread-1",
            "prepare the release",
            "installation-1",
            "2026-08-22T00:00:00Z",
            || "evt-1".to_string(),
        ))
        .expect("append work_created");
    service
        .append_event(work_linked_event(
            &work_id,
            "thread-1",
            WorkLinkOrigin::Adopted,
            "2026-08-22T00:01:00Z",
            || "evt-2".to_string(),
        ))
        .expect("append work_linked");

    let page = service
        .events_after("thread-1", None, 64)
        .expect("replay thread");
    let events: Vec<_> = page.events.into_iter().map(|row| row.event).collect();
    let projection = fold(&events);
    let work = projection.work(work_id.as_str()).expect("work replays");

    assert_eq!(work.revision, 2);
    assert!(
        work.related_thread_ids.is_empty(),
        "linking the primary thread must not duplicate it into related"
    );
}
