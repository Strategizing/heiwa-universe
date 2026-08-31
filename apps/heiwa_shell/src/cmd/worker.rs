//! `heiwa work run` — start one provider-owned worker inside the worktree that
//! `heiwa workspace prepare` created for a Work.
//!
//! The shell owns the only process spawn in this slice. Identity is appended
//! *before* the child exists, so a spawn that fails still leaves a record; the
//! reverse order would produce a running process no replay knows about.

use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use heiwa_evidence::{OperatorEvent, OperatorEventType, OperatorJournal};
use heiwa_session::operator::OperatorSessionService;
use heiwa_worker::{
    pane_closed_event, pane_opened_event, worker_exited_event, worker_heartbeat_event,
    worker_launched_event, PaneIdentity, PaneTail, WorkerIdentity, SCHEMA_VERSION,
};
use heiwa_workspace::WorkspacePreparedPayload;

pub struct IdentifiedExecutable {
    pub path: String,
    pub sha256: String,
}

/// Resolve an executable to a canonical path and the digest of its bytes.
///
/// A provider name is not identity: two machines' `claude` are different
/// programs, and the same machine's can change between runs. The spec requires
/// a verified worker to record executable identity, which means content, not a
/// label.
pub fn identify_executable(candidate: &Path) -> Result<IdentifiedExecutable> {
    let canonical = candidate
        .canonicalize()
        .map_err(|error| anyhow!("cannot resolve executable {}: {error}", candidate.display()))?;
    let bytes = std::fs::read(&canonical)
        .map_err(|error| anyhow!("cannot read executable {}: {error}", canonical.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(IdentifiedExecutable {
        path: canonical.display().to_string(),
        sha256: format!("{:x}", hasher.finalize()),
    })
}

/// Worker IDs land in journal payloads and lease capability strings, so they
/// are restricted to characters that cannot escape either.
pub fn new_worker_id(new_uuid: impl FnOnce() -> String) -> String {
    format!("worker-{}", safe_suffix(new_uuid()))
}

/// Every process invocation gets its own durable run identity. The prepared
/// worker remains stable because it owns the workspace lease.
pub fn new_run_id(new_uuid: impl FnOnce() -> String) -> String {
    format!("run-{}", safe_suffix(new_uuid()))
}

/// Pane IDs share the worker's constraints for the same reason.
pub fn new_pane_id(new_uuid: impl FnOnce() -> String) -> String {
    format!("pane-{}", safe_suffix(new_uuid()))
}

fn safe_suffix(raw: String) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect()
}

/// Find `name` on `PATH` when it has no path separator, otherwise take it as
/// given.
fn resolve_on_path(name: &str) -> Option<PathBuf> {
    if name.contains(std::path::MAIN_SEPARATOR) {
        return Some(PathBuf::from(name));
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| candidate.is_file())
}

/// The environment a worker receives.
///
/// The spec gives workers only task-required environment values. Inheriting the
/// operator's environment would hand every credential in it to a provider-owned
/// process, so the child starts from empty and is given back only what a shell
/// needs to function.
const WORKER_ENV_ALLOWLIST: [&str; 4] = ["PATH", "HOME", "LANG", "TERM"];

pub fn run(args: &[String]) -> Result<()> {
    let separator = args.iter().position(|arg| arg == "--");
    let (head, command) = match separator {
        Some(index) => (&args[..index], &args[index + 1..]),
        None => (args, &[] as &[String]),
    };
    let json_output = head.iter().any(|arg| arg == "--json");
    if command.is_empty() {
        return Err(anyhow!(
            "usage: heiwa work run <work-id> [--provider <name>] [--json] -- <command> [args...]"
        ));
    }

    let work_id = head
        .iter()
        .find(|arg| !arg.starts_with("--"))
        .ok_or_else(|| anyhow!("usage: heiwa work run <work-id> -- <command> [args...]"))?;
    let provider = flag_value(head, "--provider").unwrap_or_else(|| "local".to_string());

    let paths = heiwa_config::HeiwaPaths::resolve();
    let identity = heiwa_identity::load_from(&paths.runtime_root)
        .map_err(|error| anyhow!("{error}"))?
        .ok_or_else(|| anyhow!("no local identity on this installation; run first-run setup"))?;

    let outcome = run_in_prepared_workspace_with_output(
        &paths.evidence_dir,
        work_id,
        &identity.installation_id,
        &provider,
        command,
        !json_output,
    )?;

    if json_output {
        println!("{outcome}");
    } else {
        println!(
            "{} ran {}",
            outcome["worker_id"].as_str().unwrap_or("?"),
            outcome["executable_path"].as_str().unwrap_or("?")
        );
        println!("  cwd    {}", outcome["cwd"].as_str().unwrap_or("?"));
        println!("  branch {}", outcome["branch"].as_str().unwrap_or("?"));
        println!("  pane   {}", outcome["pane_id"].as_str().unwrap_or("?"));
        match outcome["exit_code"].as_i64() {
            Some(code) => println!("  exit   {code}"),
            None => println!("  exit   (signalled)"),
        }
    }
    Ok(())
}

fn flag_value(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|arg| arg == flag)
        .and_then(|index| args.get(index + 1))
        .cloned()
}

/// The whole run: adopt the prepared workspace, append identity, spawn, stream,
/// reap, and record the ending.
#[cfg(test)]
pub(crate) fn run_in_prepared_workspace(
    evidence_root: &Path,
    work_id: &str,
    installation_id: &str,
    provider: &str,
    command: &[String],
) -> Result<Value> {
    run_in_prepared_workspace_with_output(
        evidence_root,
        work_id,
        installation_id,
        provider,
        command,
        true,
    )
}

fn run_in_prepared_workspace_with_output(
    evidence_root: &Path,
    work_id: &str,
    installation_id: &str,
    provider: &str,
    command: &[String],
    echo_live_output: bool,
) -> Result<Value> {
    let work = crate::cmd::work::find(evidence_root, work_id)?
        .ok_or_else(|| anyhow!("Work {work_id} does not exist on this installation"))?;
    if work.origin_installation_id != installation_id {
        return Err(anyhow!(
            "Work {work_id} belongs to installation {}, not {installation_id}",
            work.origin_installation_id
        ));
    }

    let prepared = latest_prepared(evidence_root, work_id)?.ok_or_else(|| {
        anyhow!("Work {work_id} has no prepared workspace; run `heiwa workspace prepare {work_id}` first")
    })?;
    // A1-b journals written before A1-c2 carry no worker. Refuse rather than
    // invent one: the lease those events took names a different session, and a
    // run attributed to an identity no lease knows about is not verifiable.
    let worker_id = prepared.worker_id.clone().ok_or_else(|| {
        anyhow!(
            "Work {work_id} was prepared before workers were bound to leases; \
             release and re-prepare it to run a worker"
        )
    })?;
    let lease_id = prepared.lease_id.clone().unwrap_or_default();

    let executable = resolve_on_path(&command[0])
        .ok_or_else(|| anyhow!("cannot find executable on PATH: {}", command[0]))?;
    let identified = identify_executable(&executable)?;

    let worktree = PathBuf::from(&prepared.worktree_path);
    if !worktree.is_dir() {
        return Err(anyhow!(
            "prepared worktree {} no longer exists; re-prepare Work {work_id}",
            prepared.worktree_path
        ));
    }

    let now = chrono::Utc::now().to_rfc3339();
    let run_id = new_run_id(|| uuid::Uuid::new_v4().to_string());
    let worker = WorkerIdentity {
        schema_version: SCHEMA_VERSION,
        worker_id: worker_id.clone(),
        work_id: work_id.to_string(),
        thread_id: work.primary_thread_id.clone(),
        provider: provider.to_string(),
        provider_session_ref: None,
        executable_path: identified.path.clone(),
        executable_sha256: identified.sha256.clone(),
        cwd: prepared.worktree_path.clone(),
        repo_root: prepared.repo_root.clone(),
        branch: prepared.branch.clone(),
        base_commit: prepared.base_commit.clone(),
        lease_id,
        installation_id: installation_id.to_string(),
        started_at: now.clone(),
    };

    let service = service(evidence_root)?;

    // Identity first. A spawn that then fails still has a record; the reverse
    // order would leave a running process no replay knows about.
    service
        .append_event(worker_launched_event(&worker, &run_id, &now, new_event_id))
        .map_err(|error| anyhow!("{error}"))?;

    let pane = PaneIdentity {
        schema_version: SCHEMA_VERSION,
        pane_id: new_pane_id(|| uuid::Uuid::new_v4().to_string()),
        work_id: work_id.to_string(),
        worker_id: worker_id.clone(),
        cwd: prepared.worktree_path.clone(),
        repo_root: prepared.repo_root.clone(),
        branch: prepared.branch.clone(),
        opened_at: now.clone(),
    };
    service
        .append_event(pane_opened_event(
            &pane,
            &run_id,
            &worker.thread_id,
            &now,
            new_event_id,
        ))
        .map_err(|error| anyhow!("{error}"))?;

    let mut builder = Command::new(&executable);
    builder
        .args(&command[1..])
        .current_dir(&worktree)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env_clear();
    for key in WORKER_ENV_ALLOWLIST {
        if let Some(value) = std::env::var_os(key) {
            builder.env(key, value);
        }
    }

    let child = match builder.spawn() {
        Ok(child) => child,
        Err(error) => {
            let failed_at = chrono::Utc::now().to_rfc3339();
            service
                .append_event(worker_exited_event(
                    &worker,
                    &run_id,
                    None,
                    Some("spawn_failed".to_string()),
                    &failed_at,
                    new_event_id,
                ))
                .map_err(|append| anyhow!("{append}"))?;
            return Err(anyhow!("cannot start {}: {error}", identified.path));
        }
    };

    // The child exists, so the worker is live rather than merely declared.
    let pid = child.id();
    service
        .append_event(worker_heartbeat_event(
            &worker,
            &run_id,
            pid,
            &chrono::Utc::now().to_rfc3339(),
            new_event_id,
        ))
        .map_err(|error| anyhow!("{error}"))?;

    let (tail, status) = stream_and_reap(child, echo_live_output)?;

    let closed_at = chrono::Utc::now().to_rfc3339();
    service
        .append_event(pane_closed_event(
            &pane,
            &run_id,
            &worker.thread_id,
            tail.lines(),
            tail.dropped_lines(),
            &closed_at,
            new_event_id,
        ))
        .map_err(|error| anyhow!("{error}"))?;

    let exit_code = status.code();
    let failure_code = match exit_code {
        Some(0) => None,
        Some(_) => Some("nonzero_exit".to_string()),
        None => Some("signalled".to_string()),
    };
    service
        .append_event(worker_exited_event(
            &worker,
            &run_id,
            exit_code,
            failure_code.clone(),
            &closed_at,
            new_event_id,
        ))
        .map_err(|error| anyhow!("{error}"))?;

    Ok(json!({
        "work_id": work_id,
        "run_id": run_id,
        "worker_id": worker.worker_id,
        "pane_id": pane.pane_id,
        "provider": worker.provider,
        "executable_path": worker.executable_path,
        "executable_sha256": worker.executable_sha256,
        "cwd": worker.cwd,
        "repo_root": worker.repo_root,
        "branch": worker.branch,
        "base_commit": worker.base_commit,
        "lease_id": worker.lease_id,
        "pid": pid,
        "exit_code": exit_code,
        "failure_code": failure_code,
        "pane_tail": tail.lines(),
        "pane_dropped_lines": tail.dropped_lines(),
    }))
}

/// Drain both pipes concurrently and wait for the child.
///
/// Reading them in sequence deadlocks as soon as the child fills the pipe we
/// are not reading, which any real provider CLI will do.
fn stream_and_reap(
    mut child: Child,
    echo_live_output: bool,
) -> Result<(PaneTail, std::process::ExitStatus)> {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let (tx, rx) = mpsc::channel::<String>();

    let out_tx = tx.clone();
    let out_thread = std::thread::spawn(move || forward_lines(stdout, out_tx));
    let err_thread = std::thread::spawn(move || forward_lines(stderr, tx));

    let mut tail = PaneTail::default();
    for line in rx {
        // Human mode stays live. Machine-readable mode keeps stdout as one
        // JSON document while retaining the same bounded pane evidence.
        if echo_live_output {
            println!("{line}");
        }
        tail.push(&line);
    }

    // Join both readers and reap the child before returning any failure. This
    // prevents a capture failure from becoming a clean run without leaving a
    // second reader or child unreaped on the error path.
    let stdout_result = join_reader(out_thread, "stdout");
    let stderr_result = join_reader(err_thread, "stderr");
    let status_result = child.wait();
    stdout_result?;
    stderr_result?;
    let status = status_result?;
    Ok((tail, status))
}

fn join_reader(reader: std::thread::JoinHandle<()>, stream: &str) -> Result<()> {
    reader
        .join()
        .map_err(|_| anyhow!("{stream} reader thread panicked"))
}

fn forward_lines<R: Read + Send + 'static>(source: Option<R>, tx: mpsc::Sender<String>) {
    let Some(source) = source else {
        return;
    };
    for line in BufReader::new(source).lines() {
        match line {
            Ok(line) => {
                if tx.send(line).is_err() {
                    return;
                }
            }
            // A worker emitting invalid UTF-8 must not take the recorder down.
            Err(_) => return,
        }
    }
}

fn service(root: &Path) -> Result<OperatorSessionService> {
    Ok(OperatorSessionService::new(
        OperatorJournal::new(root.to_path_buf()).map_err(|error| anyhow!("{error}"))?,
    ))
}

fn new_event_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// The most recent `workspace_prepared` for this Work that has not been
/// released since.
fn latest_prepared(
    evidence_root: &Path,
    work_id: &str,
) -> Result<Option<WorkspacePreparedPayload>> {
    let journal =
        OperatorJournal::new(evidence_root.to_path_buf()).map_err(|error| anyhow!("{error}"))?;
    let mut prepared: Option<WorkspacePreparedPayload> = None;
    let mut cursor = None;
    loop {
        let page = journal
            .read_after(cursor.as_deref(), 500)
            .map_err(|error| anyhow!("{error}"))?;
        if page.events.is_empty() {
            break;
        }
        for row in &page.events {
            let event: &OperatorEvent = &row.event;
            if event.work_id.as_deref() != Some(work_id) {
                continue;
            }
            match event.event_type {
                OperatorEventType::WorkspacePrepared => {
                    prepared = WorkspacePreparedPayload::from_event(event);
                }
                OperatorEventType::WorkspaceReleased => {
                    prepared = None;
                }
                _ => {}
            }
        }
        cursor = page.next_cursor;
        if cursor.is_none() {
            break;
        }
    }
    Ok(prepared)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_executable_is_identified_by_path_and_content_digest() {
        let temp = tempfile::tempdir().expect("tempdir");
        let script = temp.path().join("fake-provider");
        std::fs::write(&script, b"#!/bin/sh\nexit 0\n").expect("write");

        let identified = identify_executable(&script).expect("identify");
        assert_eq!(
            identified.path,
            script.canonicalize().expect("canon").display().to_string()
        );
        assert_eq!(identified.sha256.len(), 64);
        assert!(identified.sha256.chars().all(|c| c.is_ascii_hexdigit()));

        std::fs::write(&script, b"#!/bin/sh\nexit 1\n").expect("rewrite");
        let changed = identify_executable(&script).expect("identify again");
        assert_ne!(
            changed.sha256, identified.sha256,
            "digest must follow content"
        );
    }

    #[test]
    fn a_missing_executable_is_refused_before_anything_is_appended() {
        let temp = tempfile::tempdir().expect("tempdir");
        let missing = temp.path().join("not-here");
        assert!(identify_executable(&missing).is_err());
    }

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

    /// A Work with a prepared workspace, ready for a worker.
    fn prepared_work(runtime: &Path, source: &Path) -> (PathBuf, String) {
        let evidence = runtime.join("evidence");
        let work_id = crate::cmd::work::create(&evidence, "run a worker", "install-1")
            .expect("create Work")["work_id"]
            .as_str()
            .expect("work id")
            .to_string();
        crate::cmd::workspace::prepare_for(
            runtime,
            &evidence,
            source,
            &work_id,
            "install-1",
            &new_worker_id(|| uuid::Uuid::new_v4().to_string()),
        )
        .expect("prepare");
        (evidence, work_id)
    }

    fn replay(evidence: &Path, work_id: &str) -> Vec<OperatorEvent> {
        let journal = OperatorJournal::new(evidence.to_path_buf()).expect("journal");
        let mut all = Vec::new();
        let mut cursor = None;
        loop {
            let page = journal.read_after(cursor.as_deref(), 500).expect("read");
            if page.events.is_empty() {
                break;
            }
            for row in &page.events {
                if row.event.work_id.as_deref() == Some(work_id) {
                    all.push(row.event.clone());
                }
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        all
    }

    #[test]
    fn a_worker_runs_inside_the_prepared_worktree_and_not_the_repository() {
        let source = repo();
        let runtime = tempfile::tempdir().expect("runtime");
        let (evidence, work_id) = prepared_work(runtime.path(), source.path());

        let outcome = run_in_prepared_workspace(
            &evidence,
            &work_id,
            "install-1",
            "local",
            &[
                "/bin/sh".to_string(),
                "-c".to_string(),
                "pwd -P".to_string(),
            ],
        )
        .expect("run");

        assert_eq!(outcome["exit_code"], 0);
        let printed = outcome["pane_tail"][0].as_str().expect("pwd output");
        let expected = PathBuf::from(outcome["cwd"].as_str().expect("cwd"))
            .canonicalize()
            .expect("canonical worktree");
        assert_eq!(
            PathBuf::from(printed),
            expected,
            "the worker must run in the prepared worktree, not the repository root"
        );
        assert_ne!(
            PathBuf::from(printed),
            source.path().canonicalize().expect("canonical source"),
            "the worker must not run in the source repository"
        );
    }

    #[test]
    fn a_run_appends_its_five_events_in_order_all_scoped_to_the_work() {
        let source = repo();
        let runtime = tempfile::tempdir().expect("runtime");
        let (evidence, work_id) = prepared_work(runtime.path(), source.path());

        run_in_prepared_workspace(
            &evidence,
            &work_id,
            "install-1",
            "local",
            &[
                "/bin/sh".to_string(),
                "-c".to_string(),
                "echo hello".to_string(),
            ],
        )
        .expect("run");

        let worker_events: Vec<_> = replay(&evidence, &work_id)
            .into_iter()
            .filter(|event| {
                matches!(
                    event.event_type,
                    OperatorEventType::WorkerLaunched
                        | OperatorEventType::WorkerHeartbeat
                        | OperatorEventType::WorkerExited
                        | OperatorEventType::PaneOpened
                        | OperatorEventType::PaneClosed
                )
            })
            .collect();

        let order: Vec<_> = worker_events.iter().map(|e| e.event_type.clone()).collect();
        assert_eq!(
            order,
            vec![
                OperatorEventType::WorkerLaunched,
                OperatorEventType::PaneOpened,
                OperatorEventType::WorkerHeartbeat,
                OperatorEventType::PaneClosed,
                OperatorEventType::WorkerExited,
            ],
            "identity is appended before the process exists, and the ending after it"
        );
        for event in &worker_events {
            assert_eq!(
                event.work_id.as_deref(),
                Some(work_id.as_str()),
                "{:?} must carry Work scope on the envelope",
                event.event_type
            );
            assert_eq!(event.actor.kind, "worker");
        }
    }

    #[test]
    fn a_completed_run_folds_to_one_exited_row_with_a_content_digest() {
        let source = repo();
        let runtime = tempfile::tempdir().expect("runtime");
        let (evidence, work_id) = prepared_work(runtime.path(), source.path());

        let outcome = run_in_prepared_workspace(
            &evidence,
            &work_id,
            "install-1",
            "local",
            &[
                "/bin/sh".to_string(),
                "-c".to_string(),
                "echo hello".to_string(),
            ],
        )
        .expect("run");

        let runs = heiwa_worker::fold_runs(&replay(&evidence, &work_id), &work_id);
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        assert_eq!(run.worker_state, heiwa_worker::WorkerState::Exited);
        assert_eq!(run.exit_code, Some(0));
        assert_eq!(run.cwd.as_deref(), outcome["cwd"].as_str());
        assert_eq!(
            run.executable_sha256.as_deref().map(str::len),
            Some(64),
            "a verified worker records what it actually opened"
        );
        assert_eq!(run.pane_tail, vec!["hello".to_string()]);
        assert_eq!(run.pane_state, Some(heiwa_worker::PaneState::Done));
        // The lease the workspace took names this same worker.
        assert_eq!(run.lease_id.is_some(), true);
    }

    #[test]
    fn two_invocations_in_one_prepared_workspace_remain_two_run_history_rows() {
        let source = repo();
        let runtime = tempfile::tempdir().expect("runtime");
        let (evidence, work_id) = prepared_work(runtime.path(), source.path());

        for output in ["first", "second"] {
            run_in_prepared_workspace(
                &evidence,
                &work_id,
                "install-1",
                "local",
                &[
                    "/bin/sh".to_string(),
                    "-c".to_string(),
                    format!("echo {output}"),
                ],
            )
            .expect("run");
        }

        let runs = heiwa_worker::fold_runs(&replay(&evidence, &work_id), &work_id);
        assert_eq!(
            runs.len(),
            2,
            "each process invocation must remain independently inspectable"
        );
        assert_eq!(runs[0].pane_tail, vec!["first".to_string()]);
        assert_eq!(runs[1].pane_tail, vec!["second".to_string()]);

        let snapshot =
            crate::cmd::work::session(&evidence, &work_id, "test-epoch").expect("Work session");
        assert_eq!(
            snapshot.collections["runs"].len(),
            2,
            "canonical Work projection must preserve both run rows too"
        );
    }

    #[test]
    fn a_work_with_no_prepared_workspace_is_refused_before_any_spawn() {
        let runtime = tempfile::tempdir().expect("runtime");
        let evidence = runtime.path().join("evidence");
        let work_id = crate::cmd::work::create(&evidence, "unprepared", "install-1")
            .expect("create Work")["work_id"]
            .as_str()
            .expect("work id")
            .to_string();

        let error = run_in_prepared_workspace(
            &evidence,
            &work_id,
            "install-1",
            "local",
            &["/bin/sh".to_string(), "-c".to_string(), "true".to_string()],
        )
        .expect_err("must refuse");
        assert!(
            error.to_string().contains("no prepared workspace"),
            "got: {error}"
        );

        // Nothing was appended: refusing before the first row is the contract.
        let runs = heiwa_worker::fold_runs(&replay(&evidence, &work_id), &work_id);
        assert!(runs.is_empty());
    }

    #[test]
    fn a_failing_worker_records_a_nonzero_exit_rather_than_vanishing() {
        let source = repo();
        let runtime = tempfile::tempdir().expect("runtime");
        let (evidence, work_id) = prepared_work(runtime.path(), source.path());

        let outcome = run_in_prepared_workspace(
            &evidence,
            &work_id,
            "install-1",
            "local",
            &[
                "/bin/sh".to_string(),
                "-c".to_string(),
                "exit 3".to_string(),
            ],
        )
        .expect("run completes even though the worker failed");

        assert_eq!(outcome["exit_code"], 3);
        let runs = heiwa_worker::fold_runs(&replay(&evidence, &work_id), &work_id);
        assert_eq!(runs[0].worker_state, heiwa_worker::WorkerState::Failed);
        assert_eq!(runs[0].failure_code.as_deref(), Some("nonzero_exit"));
    }

    #[test]
    fn a_worker_does_not_inherit_the_operator_environment() {
        let source = repo();
        let runtime = tempfile::tempdir().expect("runtime");
        let (evidence, work_id) = prepared_work(runtime.path(), source.path());

        std::env::set_var("HEIWA_TEST_FAKE_CREDENTIAL", "super-secret");
        let outcome = run_in_prepared_workspace(
            &evidence,
            &work_id,
            "install-1",
            "local",
            &[
                "/bin/sh".to_string(),
                "-c".to_string(),
                "echo \"[${HEIWA_TEST_FAKE_CREDENTIAL:-absent}]\"".to_string(),
            ],
        )
        .expect("run");
        std::env::remove_var("HEIWA_TEST_FAKE_CREDENTIAL");

        assert_eq!(
            outcome["pane_tail"][0].as_str(),
            Some("[absent]"),
            "a worker receives only task-required environment values"
        );
    }

    #[test]
    fn worker_and_pane_ids_are_safe_path_and_ref_components() {
        let worker = new_worker_id(|| "3f2b0c18-0000-4000-8000-000000000000".to_string());
        assert!(worker.starts_with("worker-"));
        let pane = new_pane_id(|| "../../etc/passwd".to_string());
        assert!(pane.starts_with("pane-"));
        for id in [worker, pane] {
            assert!(!id.contains('/'), "{id}");
            assert!(!id.contains(".."), "{id}");
            assert!(id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        }
    }

    #[test]
    fn a_reader_thread_panic_is_reported_instead_of_becoming_a_clean_run() {
        let reader = std::thread::spawn(|| panic!("injected reader failure"));
        let error = join_reader(reader, "stdout").expect_err("panic must propagate");
        assert_eq!(error.to_string(), "stdout reader thread panicked");
    }
}
