//! `heiwa workspace` — what a Work is allowed to touch on disk.
//!
//! Every mutation goes through `heiwa_workspace`, which refuses the git
//! commands that discard uncommitted work. This module resolves the runtime
//! root once and passes paths down; it holds no policy of its own.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use heiwa_evidence::{JsonlTransport, OperatorJournal};
use heiwa_session::operator::OperatorSessionService;
use heiwa_workspace::{
    acquire_writer_lease, create_worktree_in, diff_projection_in, remove_worktree_in,
    revoke_writer_lease, snapshot_in, workspace_prepared_event, WorktreeHandle, WriterLease,
};

/// Where Heiwa keeps the worktrees it owns.
fn holding_dir(runtime_root: &Path) -> PathBuf {
    runtime_root.join("worktrees")
}

pub fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("status") | None => status(args),
        Some("prepare") => prepare(&args[1..]),
        Some("--help") | Some("-h") => {
            print_help();
            Ok(())
        }
        Some(other) => Err(anyhow!("unknown workspace command: {other}")),
    }
}

fn print_help() {
    println!("heiwa workspace — what a Work may touch on disk");
    println!();
    println!("  heiwa workspace status [--json]                 this repository, unchanged");
    println!(
        "  heiwa workspace prepare <work_id> [--json]      take an isolated worktree and the write lease"
    );
}

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

fn status(args: &[String]) -> Result<()> {
    let cwd = std::env::current_dir()?;
    let status = status_for(&cwd)?;
    if has_flag(args, "--json") {
        println!("{status}");
        return Ok(());
    }
    println!(
        "{}  {}",
        status["root"].as_str().unwrap_or("?"),
        status["branch"].as_str().unwrap_or("(detached)")
    );
    println!("  HEAD {}", status["head"].as_str().unwrap_or("?"));
    let dirty = status["dirty_paths"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if dirty.is_empty() {
        println!("  clean");
    } else {
        println!("  {} uncommitted path(s):", dirty.len());
        for path in dirty.iter().take(20) {
            println!("    {}", path.as_str().unwrap_or("?"));
        }
    }
    Ok(())
}

/// Read a repository. Never writes.
pub(crate) fn status_for(repo_root: &Path) -> Result<Value> {
    let snapshot = snapshot_in(repo_root).map_err(|error| anyhow!("{error}"))?;
    Ok(json!({
        "root": snapshot.root,
        "branch": snapshot.branch,
        "head": snapshot.head,
        "remote": snapshot.remote,
        "dirty": snapshot.dirty,
        "dirty_paths": snapshot.dirty_paths,
    }))
}

fn prepare(args: &[String]) -> Result<()> {
    let work_id = args
        .iter()
        .find(|arg| !arg.starts_with("--"))
        .ok_or_else(|| anyhow!("usage: heiwa workspace prepare <work_id>"))?;
    let paths = heiwa_config::HeiwaPaths::resolve();
    let identity = heiwa_identity::load_from(&paths.runtime_root)
        .map_err(|error| anyhow!("{error}"))?
        .ok_or_else(|| anyhow!("no local identity on this installation; run first-run setup"))?;
    let cwd = std::env::current_dir()?;

    // One prepared workspace serves one worker: the lease it takes is
    // exclusive, so minting the identity here and recording it on
    // `workspace_prepared` lets `heiwa work run` adopt it instead of inventing
    // a second, conflicting one.
    let worker_id = crate::cmd::worker::new_worker_id(|| uuid::Uuid::new_v4().to_string());
    let prepared = prepare_for(
        &paths.runtime_root,
        &paths.evidence_dir,
        &cwd,
        work_id,
        &identity.installation_id,
        &worker_id,
    )?;
    if has_flag(args, "--json") {
        println!("{prepared}");
    } else {
        println!(
            "{} holds {}",
            work_id,
            prepared["repo_root"].as_str().unwrap_or("?")
        );
        println!(
            "  worktree {}",
            prepared["worktree_path"].as_str().unwrap_or("?")
        );
        println!("  branch   {}", prepared["branch"].as_str().unwrap_or("?"));
        println!(
            "  base     {}",
            prepared["base_commit"].as_str().unwrap_or("?")
        );
    }
    Ok(())
}

/// Take the lease, then the worktree. Lease first, so a refused repository
/// never leaves a stray directory behind.
pub(crate) fn prepare_for(
    runtime_root: &Path,
    evidence_root: &Path,
    repo_root: &Path,
    work_id: &str,
    installation_id: &str,
    worker_id: &str,
) -> Result<Value> {
    let work = crate::cmd::work::find(evidence_root, work_id)?
        .ok_or_else(|| anyhow!("Work {work_id} does not exist on this installation"))?;
    if work.origin_installation_id != installation_id {
        return Err(anyhow!(
            "Work {work_id} belongs to installation {}, not {installation_id}",
            work.origin_installation_id
        ));
    }

    let snapshot = snapshot_in(repo_root).map_err(|error| anyhow!("{error}"))?;
    std::fs::create_dir_all(evidence_root)?;
    let transport =
        JsonlTransport::new(evidence_root.to_path_buf()).map_err(|error| anyhow!("{error}"))?;

    let now = chrono::Utc::now();
    let expires = now + chrono::Duration::hours(8);

    let lease = acquire_writer_lease(
        evidence_root,
        &transport,
        work_id,
        &snapshot.root,
        installation_id,
        worker_id,
        &now.to_rfc3339(),
        &expires.to_rfc3339(),
        || uuid::Uuid::new_v4().to_string(),
    )
    .map_err(|error| anyhow!("{error}"))?;

    let holding = holding_dir(runtime_root);
    std::fs::create_dir_all(&holding)?;
    let handle = match create_worktree_in(repo_root, &holding, work_id) {
        Ok(handle) => handle,
        Err(error) => {
            return Err(compensate_failed_prepare(
                &transport,
                &lease,
                repo_root,
                None,
                anyhow!("{error}"),
            ));
        }
    };

    let diff = match diff_projection_in(Path::new(&handle.path), 200) {
        Ok(diff) => diff,
        Err(error) => {
            return Err(compensate_failed_prepare(
                &transport,
                &lease,
                repo_root,
                Some(&handle),
                anyhow!("{error}"),
            ));
        }
    };

    let service = OperatorSessionService::new(
        OperatorJournal::new(evidence_root.to_path_buf()).map_err(|error| anyhow!("{error}"))?,
    );
    let occurred_at = chrono::Utc::now().to_rfc3339();
    if let Err(error) = service.append_event(workspace_prepared_event(
        work_id,
        &work.primary_thread_id,
        &snapshot.root,
        &handle,
        worker_id,
        &lease.lease_id,
        &occurred_at,
        || uuid::Uuid::new_v4().to_string(),
    )) {
        return Err(compensate_failed_prepare(
            &transport,
            &lease,
            repo_root,
            Some(&handle),
            anyhow!("{error}"),
        ));
    }

    Ok(json!({
        "work_id": work_id,
        "repo_root": snapshot.root,
        "worktree_path": handle.path,
        "branch": handle.branch,
        "base_commit": handle.base_commit,
        "worker_id": worker_id,
        "lease_id": lease.lease_id,
        "source_dirty": snapshot.dirty,
        "source_dirty_paths": snapshot.dirty_paths,
        "changed_files": diff.total_files,
    }))
}

fn compensate_failed_prepare(
    transport: &JsonlTransport,
    lease: &WriterLease,
    repo_root: &Path,
    handle: Option<&WorktreeHandle>,
    cause: anyhow::Error,
) -> anyhow::Error {
    let mut cleanup_failures = Vec::new();
    if let Some(handle) = handle {
        if let Err(error) = remove_worktree_in(repo_root, handle) {
            cleanup_failures.push(format!("worktree cleanup failed: {error}"));
        }
    }
    let revoked_at = chrono::Utc::now().to_rfc3339();
    if let Err(error) = revoke_writer_lease(
        transport,
        lease,
        &revoked_at,
        "WORKSPACE_PREPARE_FAILED",
        &cause.to_string(),
    ) {
        cleanup_failures.push(format!("lease compensation failed: {error}"));
    }

    if cleanup_failures.is_empty() {
        cause
    } else {
        anyhow!(
            "workspace preparation failed: {cause}; {}",
            cleanup_failures.join("; ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path();
        heiwa_workspace::git(path, &["init", "-q", "-b", "main", "."]).expect("init");
        heiwa_workspace::git(path, &["config", "user.email", "t@heiwa.ltd"]).expect("email");
        heiwa_workspace::git(path, &["config", "user.name", "t"]).expect("name");
        std::fs::write(path.join("a.txt"), "one\n").expect("write");
        heiwa_workspace::git(path, &["add", "."]).expect("add");
        heiwa_workspace::git(path, &["commit", "-qm", "one"]).expect("commit");
        dir
    }

    fn evidence(runtime: &Path) -> PathBuf {
        runtime.join("evidence")
    }

    fn work(runtime: &Path) -> String {
        crate::cmd::work::create(&evidence(runtime), "prepare repository work", "install-1")
            .expect("create Work")["work_id"]
            .as_str()
            .expect("work id")
            .to_string()
    }

    fn prepare_test(runtime: &Path, repo: &Path, work_id: &str) -> Result<Value> {
        prepare_for(
            runtime,
            &evidence(runtime),
            repo,
            work_id,
            "install-1",
            "worker-1",
        )
    }

    #[test]
    fn status_reports_a_repository_without_changing_it() {
        let source = repo();
        let status = status_for(source.path()).expect("status");

        assert_eq!(status["branch"], "main");
        assert_eq!(status["dirty"], false);
        assert!(status["head"].as_str().expect("head").len() == 40);
    }

    #[test]
    fn status_surfaces_uncommitted_paths() {
        let source = repo();
        std::fs::write(source.path().join("a.txt"), "one\ntwo\n").expect("edit");
        let status = status_for(source.path()).expect("status");

        assert_eq!(status["dirty"], true);
        assert_eq!(
            status["dirty_paths"].as_array().expect("paths").len(),
            1,
            "the operator must see exactly what is uncommitted: {status}"
        );
    }

    #[test]
    fn preparing_an_unknown_work_is_refused_before_any_hold_is_taken() {
        let source = repo();
        let runtime = tempfile::tempdir().expect("runtime");

        let error = prepare_test(runtime.path(), source.path(), "work-missing")
            .expect_err("a workspace must belong to durable Work");

        assert!(error.to_string().contains("does not exist"), "{error}");
        let view = heiwa_evidence::WorkerStateView::replay(&evidence(runtime.path()))
            .expect("replay leases");
        assert!(view.leases.is_empty(), "no lease may be taken: {view:?}");
        assert!(
            !holding_dir(runtime.path()).join("work-missing").exists(),
            "no worktree may be created"
        );
    }

    #[test]
    fn preparing_a_workspace_returns_the_worktree_it_created() {
        let source = repo();
        let runtime = tempfile::tempdir().expect("runtime");
        let work_id = work(runtime.path());
        let prepared = prepare_test(runtime.path(), source.path(), &work_id).expect("prepare");

        assert_eq!(prepared["work_id"], work_id);
        let worktree = prepared["worktree_path"].as_str().expect("path");
        assert!(
            std::path::Path::new(worktree).join("a.txt").exists(),
            "{prepared}"
        );
    }

    #[test]
    fn successful_preparation_records_the_workspace_on_its_work() {
        let source = repo();
        let runtime = tempfile::tempdir().expect("runtime");
        let work_id = work(runtime.path());

        prepare_test(runtime.path(), source.path(), &work_id).expect("prepare");

        let journal =
            heiwa_evidence::OperatorJournal::new(evidence(runtime.path())).expect("journal");
        let page = journal.read_after(None, 64).expect("replay");
        let prepared = page
            .events
            .iter()
            .find(|row| {
                row.event.event_type == heiwa_evidence::OperatorEventType::WorkspacePrepared
            })
            .expect("workspace_prepared event");
        assert_eq!(prepared.event.work_id.as_deref(), Some(work_id.as_str()));
        let expected_path = holding_dir(runtime.path())
            .join(&work_id)
            .canonicalize()
            .expect("created worktree has a canonical path");
        assert_eq!(
            prepared.event.payload["worktree_path"],
            serde_json::Value::String(expected_path.display().to_string())
        );
    }

    #[test]
    fn preparing_a_repository_twice_is_refused_with_the_holder_named() {
        let source = repo();
        let runtime = tempfile::tempdir().expect("runtime");
        let first_work = work(runtime.path());
        let second_work = work(runtime.path());
        prepare_test(runtime.path(), source.path(), &first_work).expect("first");

        let error = prepare_test(runtime.path(), source.path(), &second_work)
            .expect_err("one writer per repository");
        assert!(
            error.to_string().contains(&first_work),
            "the refusal must name the holder: {error}"
        );
    }

    #[test]
    fn failed_worktree_creation_does_not_leave_the_repository_leased() {
        let source = repo();
        let runtime = tempfile::tempdir().expect("runtime");
        let failed_work = work(runtime.path());
        let successful_work = work(runtime.path());
        let failed_branch = format!("heiwa/{failed_work}");
        heiwa_workspace::git(source.path(), &["branch", &failed_branch, "HEAD"])
            .expect("pre-existing branch forces worktree creation to fail");

        prepare_test(runtime.path(), source.path(), &failed_work)
            .expect_err("the worktree branch already exists");

        prepare_test(runtime.path(), source.path(), &successful_work)
            .expect("a failed preparation must give the repository back");

        let view = heiwa_evidence::WorkerStateView::replay(&evidence(runtime.path()))
            .expect("replay leases");
        let failed = view
            .leases
            .values()
            .find(|lease| lease.task_id == failed_work)
            .expect("failed lease remains as evidence");
        assert_eq!(failed.status, "revoked");
        assert_eq!(
            failed.failure_code.as_deref(),
            Some("WORKSPACE_PREPARE_FAILED")
        );
        assert_eq!(
            view.leases
                .values()
                .filter(|lease| matches!(lease.status.as_str(), "issued" | "acked"))
                .count(),
            1,
            "only the successful preparation may remain live"
        );
    }
}
