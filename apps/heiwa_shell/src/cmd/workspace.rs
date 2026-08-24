//! `heiwa workspace` — what a Work is allowed to touch on disk.
//!
//! Every mutation goes through `heiwa_workspace`, which refuses the git
//! commands that discard uncommitted work. This module resolves the runtime
//! root once and passes paths down; it holds no policy of its own.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use heiwa_evidence::JsonlTransport;
use heiwa_workspace::{acquire_writer_lease, create_worktree_in, diff_projection_in, snapshot_in};

/// Where Heiwa keeps the worktrees it owns.
fn holding_dir(runtime_root: &Path) -> PathBuf {
    runtime_root.join("worktrees")
}

fn evidence_dir(runtime_root: &Path) -> PathBuf {
    runtime_root.join("evidence")
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
    let runtime_root = crate::home::heiwa_runtime_dir();
    let identity = heiwa_identity::load_from(&runtime_root)
        .map_err(|error| anyhow!("{error}"))?
        .ok_or_else(|| anyhow!("no local identity on this installation; run first-run setup"))?;
    let cwd = std::env::current_dir()?;

    let prepared = prepare_for(&runtime_root, &cwd, work_id, &identity.installation_id)?;
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
    repo_root: &Path,
    work_id: &str,
    installation_id: &str,
) -> Result<Value> {
    let snapshot = snapshot_in(repo_root).map_err(|error| anyhow!("{error}"))?;
    let evidence = evidence_dir(runtime_root);
    std::fs::create_dir_all(&evidence)?;
    let transport = JsonlTransport::new(evidence.clone()).map_err(|error| anyhow!("{error}"))?;

    let now = chrono::Utc::now();
    let expires = now + chrono::Duration::hours(8);

    acquire_writer_lease(
        &evidence,
        &transport,
        work_id,
        &snapshot.root,
        installation_id,
        &now.to_rfc3339(),
        &expires.to_rfc3339(),
        || uuid::Uuid::new_v4().to_string(),
    )
    .map_err(|error| anyhow!("{error}"))?;

    let holding = holding_dir(runtime_root);
    std::fs::create_dir_all(&holding)?;
    let handle =
        create_worktree_in(repo_root, &holding, work_id).map_err(|error| anyhow!("{error}"))?;

    let diff =
        diff_projection_in(Path::new(&handle.path), 200).map_err(|error| anyhow!("{error}"))?;

    Ok(json!({
        "work_id": work_id,
        "repo_root": snapshot.root,
        "worktree_path": handle.path,
        "branch": handle.branch,
        "base_commit": handle.base_commit,
        "source_dirty": snapshot.dirty,
        "source_dirty_paths": snapshot.dirty_paths,
        "changed_files": diff.total_files,
    }))
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
    fn preparing_a_workspace_returns_the_worktree_it_created() {
        let source = repo();
        let runtime = tempfile::tempdir().expect("runtime");
        let prepared =
            prepare_for(runtime.path(), source.path(), "work-abc", "install-1").expect("prepare");

        assert_eq!(prepared["work_id"], "work-abc");
        let worktree = prepared["worktree_path"].as_str().expect("path");
        assert!(
            std::path::Path::new(worktree).join("a.txt").exists(),
            "{prepared}"
        );
    }

    #[test]
    fn preparing_a_repository_twice_is_refused_with_the_holder_named() {
        let source = repo();
        let runtime = tempfile::tempdir().expect("runtime");
        prepare_for(runtime.path(), source.path(), "work-abc", "install-1").expect("first");

        let error = prepare_for(runtime.path(), source.path(), "work-def", "install-1")
            .expect_err("one writer per repository");
        assert!(
            error.to_string().contains("work-abc"),
            "the refusal must name the holder: {error}"
        );
    }
}
