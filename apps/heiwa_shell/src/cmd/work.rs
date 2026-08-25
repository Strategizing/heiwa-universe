//! `heiwa work` — durable Work on this installation.
//!
//! Read and create only. Work is appended through `OperatorSessionService`, so
//! this command adds no second writer; it resolves the runtime root once and
//! hands it down.

use std::path::Path;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use heiwa_evidence::OperatorJournal;
use heiwa_session::operator::OperatorSessionService;
use heiwa_work::{fold, work_created_event, Work, WorkId, WorkProjection};

pub fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("list") | Some("status") | None => list(args),
        Some("create") => create_command(&args[1..]),
        Some("--help") | Some("-h") => {
            print_help();
            Ok(())
        }
        Some(other) => Err(anyhow!("unknown work command: {other}")),
    }
}

fn print_help() {
    println!("heiwa work — durable Work on this installation");
    println!();
    println!("  heiwa work list [--json]              what Work exists and where it stands");
    println!("  heiwa work create <intent> [--json]   open a new Work and its primary thread");
}

fn service(root: &Path) -> Result<OperatorSessionService> {
    Ok(OperatorSessionService::new(
        OperatorJournal::new(root.to_path_buf()).map_err(|error| anyhow!("{error}"))?,
    ))
}

fn list(args: &[String]) -> Result<()> {
    let paths = heiwa_config::HeiwaPaths::resolve();
    let summary = summarize(&paths.evidence_dir)?;
    if has_flag(args, "--json") {
        println!("{summary}");
        return Ok(());
    }
    let works = summary["work"].as_array().cloned().unwrap_or_default();
    if works.is_empty() {
        println!("no Work on this installation yet");
        println!("  run `heiwa work create \"<what you want done>\"`");
    } else {
        for work in &works {
            println!(
                "{}  {}  rev {}",
                work["work_id"].as_str().unwrap_or("?"),
                work["status"].as_str().unwrap_or("?"),
                work["revision"].as_u64().unwrap_or(0),
            );
            println!("  {}", work["intent"].as_str().unwrap_or(""));
        }
    }
    let skipped = summary["skipped_events"].as_u64().unwrap_or(0);
    if skipped > 0 {
        println!();
        println!("! {skipped} work event(s) could not be folded; run `heiwa doctor` for detail");
    }
    Ok(())
}

fn create_command(args: &[String]) -> Result<()> {
    let intent = args
        .iter()
        .find(|arg| !arg.starts_with("--"))
        .ok_or_else(|| anyhow!("usage: heiwa work create \"<intent>\""))?;
    let paths = heiwa_config::HeiwaPaths::resolve();
    let identity = heiwa_identity::load_from(&paths.runtime_root)
        .map_err(|error| anyhow!("{error}"))?
        .ok_or_else(|| {
            anyhow!(
                "no local identity on this installation; run first-run setup before creating Work"
            )
        })?;

    let created = create(&paths.evidence_dir, intent, &identity.installation_id)?;
    if has_flag(args, "--json") {
        println!("{created}");
    } else {
        println!("opened {}", created["work_id"].as_str().unwrap_or("?"));
        println!("  {intent}");
    }
    Ok(())
}

/// Create one Work and its primary thread, atomically from the caller's view:
/// the thread exists before the event that names it.
pub(crate) fn create(root: &Path, intent: &str, installation_id: &str) -> Result<Value> {
    let service = service(root)?;
    let work_id = WorkId::generate(|| uuid::Uuid::new_v4().to_string());
    let thread_id = format!("thread-{}", uuid::Uuid::new_v4());
    service
        .ensure_thread(&thread_id)
        .map_err(|error| anyhow!("{error}"))?;

    let occurred_at = chrono::Utc::now().to_rfc3339();
    service
        .append_event(work_created_event(
            &work_id,
            &thread_id,
            intent,
            installation_id,
            &occurred_at,
            || uuid::Uuid::new_v4().to_string(),
        ))
        .map_err(|error| anyhow!("{error}"))?;

    Ok(json!({
        "work_id": work_id.as_str(),
        "primary_thread_id": thread_id,
        "intent": intent,
        "created_at": occurred_at,
    }))
}

/// Every Work visible on this installation, plus damage found while folding.
pub(crate) fn summarize(root: &Path) -> Result<Value> {
    let projection = project(root)?;
    let work: Vec<Value> = projection
        .all()
        .map(|work| {
            json!({
                "work_id": work.work_id.as_str(),
                "intent": work.intent,
                "status": work.status,
                "revision": work.revision,
                "primary_thread_id": work.primary_thread_id,
                "related_thread_ids": work.related_thread_ids,
                "origin_installation_id": work.origin_installation_id,
                "replicable": work.is_replicable(),
                "created_at": work.created_at,
                "updated_at": work.updated_at,
            })
        })
        .collect();

    Ok(json!({
        "work": work,
        "skipped_events": projection.skipped_events,
    }))
}

/// Fold the operator stream in durable append order.
///
/// Reading each thread separately destroys cross-thread ordering: a later
/// `work_linked` can be visited before its earlier `work_created`, making valid
/// history look damaged. The journal cursor is the order authority.
pub(crate) fn project(root: &Path) -> Result<WorkProjection> {
    const PAGE_SIZE: usize = 256;

    let journal = OperatorJournal::new(root.to_path_buf()).map_err(|error| anyhow!("{error}"))?;
    let mut cursor: Option<String> = None;
    let mut events = Vec::new();
    loop {
        let page = journal
            .read_after(cursor.as_deref(), PAGE_SIZE)
            .map_err(|error| anyhow!("{error}"))?;
        if page.events.is_empty() {
            break;
        }
        cursor = page.next_cursor;
        events.extend(page.events.into_iter().map(|row| row.event));
    }
    Ok(fold(&events))
}

pub(crate) fn find(root: &Path, work_id: &str) -> Result<Option<Work>> {
    Ok(project(root)?.work(work_id).cloned())
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn a_fresh_root_lists_no_work() {
        let dir = root();
        let summary = summarize(dir.path()).expect("summarize");
        assert_eq!(summary["work"].as_array().map(Vec::len), Some(0));
        assert!(summary.get("errors").is_none(), "{summary}");
    }

    #[test]
    fn creating_work_makes_it_listable_and_replayable() {
        let dir = root();
        let created =
            create(dir.path(), "prepare the release", "installation-1").expect("create work");
        let work_id = created["work_id"].as_str().expect("work_id").to_string();
        assert!(work_id.starts_with("work-"), "{work_id}");

        let summary = summarize(dir.path()).expect("summarize");
        let listed = summary["work"].as_array().expect("array");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0]["work_id"], work_id);
        assert_eq!(listed[0]["intent"], "prepare the release");
        assert_eq!(listed[0]["revision"], 1);
        assert_eq!(
            listed[0]["replicable"], false,
            "work created before enrolment must not claim mesh reach"
        );
    }

    #[test]
    fn a_damaged_work_event_is_counted_rather_than_hidden() {
        use heiwa_work::{work_linked_event, WorkLinkOrigin};

        let dir = root();
        create(dir.path(), "prepare the release", "installation-1").expect("create work");

        // A link naming a Work that was never created: real damage. Built
        // here through the same public API the command uses, so no test-only
        // helper has to exist in the production module.
        let service = service(dir.path()).expect("service");
        service.ensure_thread("thread-orphan").expect("thread");
        service
            .append_event(work_linked_event(
                &WorkId::parse("work-missing").expect("id"),
                "thread-orphan",
                WorkLinkOrigin::Minted,
                "2026-08-22T00:01:00Z",
                || "evt-orphan".to_string(),
            ))
            .expect("append orphan link");

        let summary = summarize(dir.path()).expect("summarize");
        assert_eq!(
            summary["skipped_events"], 1,
            "damage found while folding must reach the surface: {summary}"
        );
    }

    #[test]
    fn summarizing_related_threads_preserves_global_event_order() {
        use heiwa_work::{work_linked_event, WorkLinkOrigin};

        let dir = root();
        let service = service(dir.path()).expect("service");
        service.ensure_thread("thread-z-primary").expect("primary");
        service.ensure_thread("thread-a-related").expect("related");
        let work_id = WorkId::parse("work-ordered").expect("work id");
        service
            .append_event(work_created_event(
                &work_id,
                "thread-z-primary",
                "preserve event order",
                "installation-1",
                "2026-08-24T00:00:00Z",
                || "evt-created".to_string(),
            ))
            .expect("work created");
        service
            .append_event(work_linked_event(
                &work_id,
                "thread-a-related",
                WorkLinkOrigin::Adopted,
                "2026-08-24T00:01:00Z",
                || "evt-linked".to_string(),
            ))
            .expect("work linked");

        let summary = summarize(dir.path()).expect("summarize");
        assert_eq!(summary["skipped_events"], 0, "{summary}");
        assert_eq!(
            summary["work"][0]["related_thread_ids"],
            serde_json::json!(["thread-a-related"]),
            "{summary}"
        );
    }
}
