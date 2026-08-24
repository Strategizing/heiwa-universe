# Work Fabric A1-b — Workspace Coordinator Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give one `Work` a governed hold on one local repository: a recorded repository snapshot, an isolated Heiwa-owned worktree to mutate, an exclusive writer lease that does not survive a crash, and bounded diff and test projections.

**Architecture:** A new crate `heiwa_workspace` owns repository facts and worktree lifecycle. It shells out to `git` through one function, which is the crate's only process boundary; every other function is a pure fold over that output. The writer lease is **not** a new store — it rides the `worker_leases` evidence stream that `heiwa_evidence` already replays and revokes on restart. Workspace facts reach `Work` as `work_id`-scoped operator events, so they replay through the A1-a projector without a second authority.

**Tech Stack:** Rust 2021, `heiwa_evidence` (`PersistedWorkerLease`, `worker_leases`, `recover_interrupted`), `heiwa_work` (A1-a aggregate and events), `serde`, `thiserror`. The `git` binary is a runtime dependency, invoked exactly as the shell already invokes it. No new external crates.

---

## Scope

This is **plan 2 of 3** for Release A1.

| Plan | Delivers | Status |
|---|---|---|
| A1-a | Durable `Work`, canonical `work_id`, migration, projector, bounded snapshot, epoch-guarded deltas, `heiwa work` | **done** at `d14cfbb7` |
| **A1-b (this plan)** | Repository snapshot, canonical root and symlink refusal, isolated worktree, exclusive writer lease with restart revocation, dirty-tree refusal boundary, diff and test projections, `heiwa workspace` | this plan |
| A1-c | Worker + terminal pane bound to Work, tri-surface agreement, approval → receipt through the Action Gate, restart recovery, and `scripts/check_work_fabric_a1_acceptance.sh` | not started |

`scripts/check_work_fabric_a1_acceptance.sh` is still written in A1-c, when the
whole checkpoint can pass. A1-b adds ledger rows only.

**Independently useful because:** a user can point Heiwa at a repository, see
exactly what state it is in, get an isolated worktree that cannot touch their
uncommitted work, hold an exclusive writer lease that is revoked if the runtime
dies, and read a bounded diff and test result — all without GitHub, a provider
agent, or a second repository.

## Design decisions locked in here

**1. Git is a subprocess, not a library.** The repo has no `git2`/`gix`
dependency and already shells out (`apps/heiwa_shell/src/cmd/app.rs:615`).
Adding a git library for one crate would introduce a second way to talk to
repositories. One function, `git()`, is the entire process boundary.

**2. Tests run against real repositories.** `git init` in a `tempfile::tempdir()`
costs about 50ms. A hand-written fake would have to model git's output formats,
and a fake that lies is worse than no test. Every test in this plan builds a
real repository.

**3. Dirty-tree preservation is a refusal boundary, not a mechanism.**
Verified against real git while writing this plan: `git worktree add` builds the
new directory from a **commit** and leaves the user's working tree untouched —
a modified file stayed ` M` in the source repo while the worktree received the
committed content. So preservation is free. What is *not* free is refusing to
run the commands that would destroy uncommitted work. Task 6 makes that refusal
explicit and tested, and records dirty state as a visible fact.

**4. The writer lease reuses `worker_leases`.** `heiwa_evidence::recover_interrupted`
(`crates/heiwa_evidence/src/state.rs:97-130`) already replays every lease and
revokes any left in `issued` or `acked` when the runtime restarts. The spec's
**Recover** lifecycle requires exactly that ("revokes or closes unproven
leases"). Building a second lease store would mean a second thing to recover,
and one of them would eventually be forgotten.

**5. Workspace events are `work_id`-scoped operator events.** A1-a established
that `Work` is a fold over the operator journal. Workspace facts join the same
stream so a Work replays complete, rather than joining across two stores.

## Existing substrate this plan builds on

Read these before starting. Every fact below was verified at `d14cfbb7`.

- `apps/heiwa_shell/src/cmd/app.rs:615` — `git_output(repo_root, args)`, the
  established invocation idiom: `Command::new("git").args(args).current_dir(root).output()`.
  `git_is_dirty` at `:631` uses `status --porcelain=v1`. This plan generalises
  that shape into `heiwa_workspace`; it does not invent a new one.
- `crates/heiwa_evidence/src/records.rs:93` — `PersistedWorkerLease { lease_id,
  task_id, session_id, node_id, capability, status, issued_at, updated_at,
  expires_at, acked_at, completed_at, failure_code, reason }`.
- `crates/heiwa_evidence/src/journal.rs:28` — `EvidenceTransport::upsert_worker_lease`,
  which appends to the `worker_leases` stream (`:150`). `JsonlTransport` is the
  real implementation; `NoopTransport` (`:203`) swallows writes.
- `crates/heiwa_evidence/src/state.rs:97` — `recover_interrupted(dir, transport)`
  replays `WorkerStateView` and revokes every lease whose status is `issued` or
  `acked`, with reason `"runtime restarted before lease completion"` (`:119-130`).
- `crates/heiwa_evidence/src/operator.rs:101` — `OperatorEventType` is a
  **closed** enum. A1-a proved the consequence: adding a variant breaks
  `apply_to_existing_thread` in `crates/heiwa_session/src/operator.rs:1521`
  until every new variant is handled. That is a feature. Expect it.
- `crates/heiwa_session/src/operator.rs` — `requires_work_id` (added in A1-a)
  is the pattern for making an event type refuse to exist unscoped.
- `crates/heiwa_work/` — the A1-a crate. `WorkId::parse`, `fold`, and the
  `work_created_event` builder shape are the precedent for this crate's events.
- `crates/heiwa_config::HeiwaPaths` is the only per-user root resolver.
  `apps/heiwa_shell/src/home.rs::heiwa_runtime_dir()` is the shell's single call
  into it. `scripts/check_l0_acceptance.sh` **fails on a second resolver** — do
  not resolve a root inside `heiwa_workspace`.
- `crates/heiwa_mesh/` is the precedent for crate shape: `*_in(&Path, …)`
  functions, no root resolution inside the crate, injected clocks.
- `heiwa_protocol::SandboxMode::Worktree` exists with **zero consumers**
  (`crates/heiwa_protocol/src/lib.rs:337`). Do not wire it in this plan; it
  belongs to the worker contract in A1-c.

## Verified git behaviour

These were run against real git while writing this plan. The plan depends on
them; if a step surprises you, re-run these before changing the plan.

| Command | Output shape | Why it matters |
|---|---|---|
| `git rev-parse HEAD` | one 40-char sha | base commit for a worktree |
| `git branch --show-current` | branch name, or **empty** on detached HEAD | empty is not an error |
| `git status --porcelain=v1` | one line per change, `" M a.txt"`; empty when clean | dirty detection |
| `git worktree add -b <branch> <path> <commit>` | creates `<path>` from `<commit>` | leaves the source working tree untouched |
| `git worktree list --porcelain` | `worktree <path>` / `HEAD <sha>` / `branch <ref>` blocks, blank-line separated | enumerating worktrees |
| `git worktree remove <path>` | silent on success | cleanup |
| `git diff --stat <commit>` | per-file stat lines | bounded diff projection |

## File Structure

**Create:**

| Path | Responsibility |
|---|---|
| `crates/heiwa_workspace/Cargo.toml` | Manifest. Depends on `heiwa_evidence`, `heiwa_work`, `serde`, `serde_json`, `thiserror`. |
| `crates/heiwa_workspace/src/lib.rs` | Crate docs, `WorkspaceError`, re-exports. |
| `crates/heiwa_workspace/src/git.rs` | `git()` — the crate's only process boundary — and `GitError`. |
| `crates/heiwa_workspace/src/repository.rs` | `RepositorySnapshotV1`, `snapshot_in`, canonical root and symlink refusal. |
| `crates/heiwa_workspace/src/worktree.rs` | `WorktreeHandle`, `create_worktree_in`, `list_worktrees_in`, `remove_worktree_in`. |
| `crates/heiwa_workspace/src/lease.rs` | `WriterLease`, `acquire`, `release`, exclusivity, restart revocation. |
| `crates/heiwa_workspace/src/projection.rs` | `DiffProjectionV1`, `TestProjectionV1`, bounded. |
| `crates/heiwa_workspace/src/events.rs` | `workspace_prepared` / `workspace_released` builders and payloads. |
| `crates/heiwa_workspace/tests/workspace_core.rs` | Integration across the public surface, against real repositories. |
| `apps/heiwa_shell/src/cmd/workspace.rs` | `heiwa workspace prepare|status|release`. |

**Modify:**

| Path | Change |
|---|---|
| `crates/heiwa_evidence/src/operator.rs` | Add `WorkspacePrepared` / `WorkspaceReleased` to `OperatorEventType`. |
| `crates/heiwa_session/src/operator.rs` | Extend `requires_work_id`; extend the `apply_to_existing_thread` non-terminal arm. |
| `Cargo.toml` | Add `crates/heiwa_workspace` to workspace members, after `crates/heiwa_work`. |
| `apps/heiwa_shell/Cargo.toml` | Add `heiwa_workspace`. |
| `apps/heiwa_shell/src/cmd/mod.rs`, `src/cli.rs`, `src/main.rs` | Register and document `workspace`. |
| `scripts/ci_rust_test_group.sh` | Add `heiwa_workspace` to `foundation_packages`, `workspace_core` to `foundation_b_targets`. |
| `docs/superpowers/ledgers/2026-08-22-work-fabric-task-ledger.md` | Add the A1-b table. |

---

### Task 1: The single git boundary

**Files:**
- Create: `crates/heiwa_workspace/Cargo.toml`
- Create: `crates/heiwa_workspace/src/lib.rs`
- Create: `crates/heiwa_workspace/src/git.rs`
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Write the failing test**

Create `crates/heiwa_workspace/src/git.rs` containing only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// A real repository with one commit. Real git, because a fake would have
    /// to model git's output formats and a fake that lies is worse than no
    /// test at all.
    pub(crate) fn repo() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path();
        git(path, &["init", "-q", "-b", "main", "."]).expect("init");
        git(path, &["config", "user.email", "test@heiwa.ltd"]).expect("email");
        git(path, &["config", "user.name", "test"]).expect("name");
        std::fs::write(path.join("a.txt"), "one\n").expect("write");
        git(path, &["add", "."]).expect("add");
        git(path, &["commit", "-qm", "one"]).expect("commit");
        dir
    }

    #[test]
    fn a_successful_command_returns_trimmed_stdout() {
        let dir = repo();
        let head = git(dir.path(), &["rev-parse", "HEAD"]).expect("rev-parse");
        assert_eq!(head.len(), 40, "a sha and nothing else: {head:?}");
        assert!(!head.ends_with('\n'), "output must be trimmed");
    }

    #[test]
    fn a_failing_command_reports_the_stderr_git_gave() {
        let dir = repo();
        let error = git(dir.path(), &["rev-parse", "no-such-ref"])
            .expect_err("an unknown ref must fail");

        let GitError::Failed { args, stderr, .. } = &error else {
            panic!("expected a Failed variant, got {error:?}");
        };
        assert!(args.contains("no-such-ref"), "{args}");
        assert!(
            !stderr.is_empty(),
            "swallowing git's own message leaves the caller guessing"
        );
    }

    #[test]
    fn a_directory_that_is_not_a_repository_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = git(dir.path(), &["rev-parse", "HEAD"]).expect_err("not a repo");
        assert!(matches!(error, GitError::Failed { .. }));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Create the manifest so the crate exists.

`crates/heiwa_workspace/Cargo.toml`:

```toml
[package]
name = "heiwa_workspace"
version = "0.1.0"
edition = "2021"
license.workspace = true
repository.workspace = true
homepage.workspace = true
documentation.workspace = true
readme.workspace = true
keywords.workspace = true
categories.workspace = true
description = "Repository snapshots, isolated worktrees, and writer leases for Heiwa Work."

[dependencies]
heiwa_evidence = { path = "../heiwa_evidence" }
heiwa_work = { path = "../heiwa_work" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "2"

[dev-dependencies]
tempfile = "3"
```

`crates/heiwa_workspace/src/lib.rs`:

```rust
//! The Workspace Coordinator: what a `Work` is allowed to touch on disk.
//!
//! One `Work` gets a recorded view of one repository, an isolated worktree to
//! mutate, and an exclusive writer lease. The crate shells out to `git`
//! through exactly one function (`git::git`); everything else folds that
//! output into typed facts.
//!
//! Root resolution is deliberately absent. `heiwa_config::HeiwaPaths` is the
//! only resolver in the product and `scripts/check_l0_acceptance.sh` fails on
//! a second one, so every entry point here takes a path.
//!
//! See `docs/superpowers/specs/2026-08-22-heiwa-work-fabric-design.md`.

pub mod git;

pub use git::{git, GitError};
```

Add `"crates/heiwa_workspace",` to `members` in the root `Cargo.toml`,
immediately after `"crates/heiwa_work",`.

Run: `cargo test -p heiwa_workspace`
Expected: FAIL to compile — `cannot find function 'git'`.

- [ ] **Step 3: Write the boundary**

Prepend to `crates/heiwa_workspace/src/git.rs`:

```rust
//! The one place this crate starts a process.
//!
//! Concentrated deliberately: every repository fact in this crate is a fold
//! over the output of this function, so there is exactly one thing to audit
//! for argument injection, one place that decides what a failure means, and
//! one shape for callers to handle.

use std::path::Path;
use std::process::Command;

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("could not run git in {dir}: {source}")]
    Unavailable {
        dir: String,
        #[source]
        source: std::io::Error,
    },
    #[error("git {args} failed in {dir}: {stderr}")]
    Failed {
        dir: String,
        args: String,
        stderr: String,
    },
}

/// Run git in `dir` and return trimmed stdout.
///
/// Arguments are passed as a list, never through a shell, so a branch name or
/// path containing a space or a semicolon is data rather than syntax.
pub fn git(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|source| GitError::Unavailable {
            dir: dir.display().to_string(),
            source,
        })?;

    if !output.status.success() {
        return Err(GitError::Failed {
            dir: dir.display().to_string(),
            args: args.join(" "),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        });
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p heiwa_workspace`
Expected: PASS, 3 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/heiwa_workspace Cargo.toml Cargo.lock
git commit -m "feat(workspace): add the single git process boundary"
```

---

### Task 2: Repository snapshot

**Files:**
- Create: `crates/heiwa_workspace/src/repository.rs`
- Modify: `crates/heiwa_workspace/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/heiwa_workspace/src/repository.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::repo;

    #[test]
    fn a_clean_repository_reports_its_branch_head_and_cleanliness() {
        let dir = repo();
        let snapshot = snapshot_in(dir.path()).expect("snapshot");

        assert_eq!(snapshot.branch.as_deref(), Some("main"));
        assert_eq!(snapshot.head.len(), 40);
        assert!(!snapshot.dirty, "a fresh commit leaves nothing uncommitted");
        assert!(snapshot.dirty_paths.is_empty());
        assert_eq!(snapshot.remote, None, "a local repo has no origin");
    }

    #[test]
    fn an_uncommitted_change_is_recorded_as_a_visible_fact() {
        let dir = repo();
        std::fs::write(dir.path().join("a.txt"), "one\ntwo\n").expect("write");
        let snapshot = snapshot_in(dir.path()).expect("snapshot");

        assert!(snapshot.dirty);
        assert_eq!(
            snapshot.dirty_paths,
            vec!["a.txt".to_string()],
            "the user must be able to see exactly what is uncommitted"
        );
    }

    #[test]
    fn an_untracked_file_counts_as_dirty() {
        let dir = repo();
        std::fs::write(dir.path().join("new.txt"), "hello\n").expect("write");
        let snapshot = snapshot_in(dir.path()).expect("snapshot");

        assert!(snapshot.dirty, "untracked work is still the user's work");
        assert_eq!(snapshot.dirty_paths, vec!["new.txt".to_string()]);
    }

    #[test]
    fn a_detached_head_has_no_branch_and_is_not_an_error() {
        let dir = repo();
        let head = crate::git::git(dir.path(), &["rev-parse", "HEAD"]).expect("head");
        crate::git::git(dir.path(), &["checkout", "-q", "--detach", &head]).expect("detach");

        let snapshot = snapshot_in(dir.path()).expect("a detached head still snapshots");
        assert_eq!(snapshot.branch, None);
        assert_eq!(snapshot.head, head);
    }

    #[test]
    fn a_directory_that_is_not_a_repository_is_refused_by_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let error = snapshot_in(dir.path()).expect_err("not a repository");
        assert!(
            matches!(error, WorkspaceError::NotARepository { .. }),
            "the caller needs to distinguish this from a broken git: {error:?}"
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Make the git test helper reachable: in `crates/heiwa_workspace/src/git.rs`,
change `mod tests {` to `pub(crate) mod tests {`.

Add to `crates/heiwa_workspace/src/lib.rs`:

```rust
pub mod repository;
```

Run: `cargo test -p heiwa_workspace --lib repository`
Expected: FAIL to compile — `cannot find function 'snapshot_in'`.

- [ ] **Step 3: Write the snapshot**

Prepend to `crates/heiwa_workspace/src/repository.rs`:

```rust
//! What a repository is, right now, as a recorded fact.
//!
//! Every field here is read-only. Nothing in this module writes to a
//! repository, so a snapshot can never be the thing that lost someone's work.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::git::{git, GitError};
use crate::WorkspaceError;

/// Version of the recorded repository shape.
pub const REPOSITORY_SNAPSHOT_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositorySnapshotV1 {
    pub version: u32,
    /// The canonical path, with symlinks resolved.
    pub root: String,
    /// `None` on a detached HEAD, which is a state, not a failure.
    pub branch: Option<String>,
    pub head: String,
    pub remote: Option<String>,
    /// Whether anything is uncommitted, tracked or not.
    pub dirty: bool,
    /// Exactly which paths are uncommitted, so the fact is inspectable
    /// rather than a bare boolean the user has to trust.
    pub dirty_paths: Vec<String>,
}

/// Read `root` and record what it is.
pub fn snapshot_in(root: &Path) -> Result<RepositorySnapshotV1, WorkspaceError> {
    // Ask git for the repository root rather than trusting the argument: it
    // resolves symlinks and refuses a directory that only looks like a repo.
    let canonical = match git(root, &["rev-parse", "--show-toplevel"]) {
        Ok(path) => path,
        Err(GitError::Failed { .. }) => {
            return Err(WorkspaceError::NotARepository {
                path: root.display().to_string(),
            })
        }
        Err(other) => return Err(WorkspaceError::Git(other)),
    };

    let head = git(root, &["rev-parse", "HEAD"]).map_err(WorkspaceError::Git)?;

    // Empty output means a detached HEAD. Not an error, and not a branch.
    let branch = git(root, &["branch", "--show-current"])
        .map_err(WorkspaceError::Git)?;
    let branch = if branch.is_empty() { None } else { Some(branch) };

    // No origin is normal for a local repository, so a failure here is a fact
    // rather than a problem.
    let remote = git(root, &["remote", "get-url", "origin"]).ok().filter(|url| !url.is_empty());

    let status = git(root, &["status", "--porcelain=v1"]).map_err(WorkspaceError::Git)?;
    let dirty_paths = parse_status_paths(&status);

    Ok(RepositorySnapshotV1 {
        version: REPOSITORY_SNAPSHOT_VERSION,
        root: canonical,
        branch,
        head,
        remote,
        dirty: !dirty_paths.is_empty(),
        dirty_paths,
    })
}

/// Pull the paths out of `git status --porcelain=v1`.
///
/// Each line is two status characters, a space, then the path. A rename is
/// `R  old -> new`; the new path is the one that exists, so that is the one
/// reported.
fn parse_status_paths(status: &str) -> Vec<String> {
    status
        .lines()
        .filter_map(|line| {
            let path = line.get(3..)?.trim();
            if path.is_empty() {
                return None;
            }
            Some(match path.split_once(" -> ") {
                Some((_old, new)) => new.to_string(),
                None => path.to_string(),
            })
        })
        .collect()
}
```

Add to `crates/heiwa_workspace/src/lib.rs`, replacing the existing re-export
block:

```rust
pub mod git;
pub mod repository;

pub use git::{git, GitError};
pub use repository::{snapshot_in, RepositorySnapshotV1, REPOSITORY_SNAPSHOT_VERSION};

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("{path} is not a git repository")]
    NotARepository { path: String },
    #[error(transparent)]
    Git(#[from] GitError),
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p heiwa_workspace`
Expected: PASS, 8 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/heiwa_workspace
git commit -m "feat(workspace): record what a repository is without touching it"
```

---

### Task 3: Canonical roots and symlink refusal

The spec requires this directly: "Canonical roots, symlink checks, worktrees,
and writer leases prevent path escape and overlapping mutation."

**Files:**
- Create: `crates/heiwa_workspace/src/scope.rs`
- Modify: `crates/heiwa_workspace/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/heiwa_workspace/src/scope.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn rooted() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join("inside")).expect("inside");
        std::fs::write(dir.path().join("inside/file.txt"), "x").expect("file");
        dir
    }

    #[test]
    fn a_path_inside_the_root_resolves() {
        let dir = rooted();
        let resolved = resolve_in_scope(dir.path(), &dir.path().join("inside/file.txt"))
            .expect("a path under the root is in scope");
        assert!(resolved.ends_with("inside/file.txt"), "{resolved:?}");
    }

    #[test]
    fn a_traversal_out_of_the_root_is_refused() {
        let dir = rooted();
        let error = resolve_in_scope(dir.path(), &dir.path().join("inside/../../elsewhere"))
            .expect_err("../ must not leave the root");
        assert!(matches!(error, WorkspaceError::PathEscape { .. }), "{error:?}");
    }

    #[test]
    fn a_symlink_pointing_out_of_the_root_is_refused() {
        // The case a string-prefix check misses entirely: the path looks like
        // it is inside the root right up until the filesystem resolves it.
        let dir = rooted();
        let outside = tempfile::tempdir().expect("outside");
        std::fs::write(outside.path().join("secret.txt"), "s").expect("secret");

        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path().join("secret.txt"), dir.path().join("inside/link.txt"))
            .expect("symlink");

        #[cfg(unix)]
        {
            let error = resolve_in_scope(dir.path(), &dir.path().join("inside/link.txt"))
                .expect_err("a symlink out of the root is an escape");
            assert!(matches!(error, WorkspaceError::PathEscape { .. }), "{error:?}");
        }
    }

    #[test]
    fn a_path_that_does_not_exist_yet_is_judged_by_its_parent() {
        // A worktree directory is named before it is created, so the check
        // has to work on a path with no inode.
        let dir = rooted();
        let resolved = resolve_in_scope(dir.path(), &dir.path().join("inside/not-yet"))
            .expect("a new name under an existing parent is in scope");
        assert!(resolved.ends_with("inside/not-yet"), "{resolved:?}");
    }

    #[test]
    fn a_new_path_under_a_parent_outside_the_root_is_refused() {
        let dir = rooted();
        let outside = tempfile::tempdir().expect("outside");
        let error = resolve_in_scope(dir.path(), &outside.path().join("not-yet"))
            .expect_err("a new name outside the root is still outside");
        assert!(matches!(error, WorkspaceError::PathEscape { .. }), "{error:?}");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Add `pub mod scope;` to `crates/heiwa_workspace/src/lib.rs`.

Run: `cargo test -p heiwa_workspace --lib scope`
Expected: FAIL to compile — `cannot find function 'resolve_in_scope'`.

- [ ] **Step 3: Write the scope check**

Prepend to `crates/heiwa_workspace/src/scope.rs`:

```rust
//! Deciding whether a path is inside a root, after the filesystem has had its
//! say.
//!
//! A string prefix comparison is not this check. `root/link` can be a symlink
//! to anywhere on the disk, and it passes a prefix test right up until it is
//! opened. Both sides are canonicalised first so the comparison is between
//! real locations.

use std::path::{Path, PathBuf};

use crate::WorkspaceError;

/// Resolve `candidate` and confirm it lies within `root`.
///
/// `candidate` need not exist: a worktree is named before it is created. When
/// it does not exist, its nearest existing ancestor is canonicalised instead
/// and the remaining components are appended, so a new name is judged by where
/// it would actually land.
pub fn resolve_in_scope(root: &Path, candidate: &Path) -> Result<PathBuf, WorkspaceError> {
    let escape = || WorkspaceError::PathEscape {
        root: root.display().to_string(),
        path: candidate.display().to_string(),
    };

    let canonical_root = root.canonicalize().map_err(|_| escape())?;
    let resolved = canonicalize_existing_prefix(candidate).ok_or_else(escape)?;

    if resolved.starts_with(&canonical_root) {
        Ok(resolved)
    } else {
        Err(escape())
    }
}

/// Canonicalise as much of `path` as exists, then re-append the rest.
fn canonicalize_existing_prefix(path: &Path) -> Option<PathBuf> {
    if let Ok(resolved) = path.canonicalize() {
        return Some(resolved);
    }

    let mut trailing = Vec::new();
    let mut cursor = path;
    loop {
        let parent = cursor.parent()?;
        let name = cursor.file_name()?;
        trailing.push(name.to_os_string());
        if let Ok(resolved) = parent.canonicalize() {
            let mut out = resolved;
            for component in trailing.iter().rev() {
                out.push(component);
            }
            return Some(out);
        }
        cursor = parent;
    }
}
```

Add the variant to `WorkspaceError` in `crates/heiwa_workspace/src/lib.rs`:

```rust
    #[error("{path} resolves outside the permitted root {root}")]
    PathEscape { root: String, path: String },
```

and add `pub use scope::resolve_in_scope;` to the re-exports.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p heiwa_workspace`
Expected: PASS, 13 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/heiwa_workspace
git commit -m "feat(workspace): refuse path escape after the filesystem resolves it"
```

---

### Task 4: Isolated worktree lifecycle

**Files:**
- Create: `crates/heiwa_workspace/src/worktree.rs`
- Modify: `crates/heiwa_workspace/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/heiwa_workspace/src/worktree.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::repo;

    #[test]
    fn a_worktree_is_created_at_the_repositorys_head() {
        let source = repo();
        let holding = tempfile::tempdir().expect("holding");
        let head = crate::git::git(source.path(), &["rev-parse", "HEAD"]).expect("head");

        let handle = create_worktree_in(source.path(), holding.path(), "work-abc")
            .expect("create worktree");

        assert_eq!(handle.base_commit, head);
        assert_eq!(handle.work_id, "work-abc");
        assert!(handle.branch.starts_with("heiwa/work-"), "{}", handle.branch);
        assert!(
            std::path::Path::new(&handle.path).join("a.txt").exists(),
            "the worktree must actually contain the tree"
        );
    }

    #[test]
    fn a_worktree_never_disturbs_uncommitted_work_in_the_source() {
        // Verified against real git while planning: `worktree add` builds from
        // a commit and leaves the source working tree alone. This test is what
        // keeps that true.
        let source = repo();
        let holding = tempfile::tempdir().expect("holding");
        std::fs::write(source.path().join("a.txt"), "one\nMINE\n").expect("dirty it");

        let handle = create_worktree_in(source.path(), holding.path(), "work-abc")
            .expect("create worktree");

        let still_there = std::fs::read_to_string(source.path().join("a.txt")).expect("read");
        assert_eq!(
            still_there, "one\nMINE\n",
            "the user's uncommitted edit must survive untouched"
        );
        let in_worktree =
            std::fs::read_to_string(std::path::Path::new(&handle.path).join("a.txt")).expect("read");
        assert_eq!(
            in_worktree, "one\n",
            "the worktree gets the committed state, not the dirty edit"
        );
    }

    #[test]
    fn two_works_get_two_separate_worktrees() {
        let source = repo();
        let holding = tempfile::tempdir().expect("holding");

        let first = create_worktree_in(source.path(), holding.path(), "work-abc").expect("first");
        let second = create_worktree_in(source.path(), holding.path(), "work-def").expect("second");

        assert_ne!(first.path, second.path);
        assert_ne!(first.branch, second.branch);
    }

    #[test]
    fn creating_a_second_worktree_for_one_work_is_refused() {
        let source = repo();
        let holding = tempfile::tempdir().expect("holding");
        create_worktree_in(source.path(), holding.path(), "work-abc").expect("first");

        let error = create_worktree_in(source.path(), holding.path(), "work-abc")
            .expect_err("one Work holds one worktree per repository");
        assert!(matches!(error, WorkspaceError::WorktreeExists { .. }), "{error:?}");
    }

    #[test]
    fn a_created_worktree_is_listed_against_its_work() {
        let source = repo();
        let holding = tempfile::tempdir().expect("holding");
        create_worktree_in(source.path(), holding.path(), "work-abc").expect("create");

        let listed = list_worktrees_in(source.path()).expect("list");
        assert!(
            listed.iter().any(|w| w.work_id.as_deref() == Some("work-abc")),
            "{listed:?}"
        );
    }

    #[test]
    fn removing_a_worktree_leaves_the_source_repository_intact() {
        let source = repo();
        let holding = tempfile::tempdir().expect("holding");
        let handle = create_worktree_in(source.path(), holding.path(), "work-abc").expect("create");

        remove_worktree_in(source.path(), &handle).expect("remove");

        assert!(!std::path::Path::new(&handle.path).exists());
        assert!(
            source.path().join("a.txt").exists(),
            "removing a worktree must never touch the source"
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Add `pub mod worktree;` to `crates/heiwa_workspace/src/lib.rs`.

Run: `cargo test -p heiwa_workspace --lib worktree`
Expected: FAIL to compile — `cannot find function 'create_worktree_in'`.

- [ ] **Step 3: Write the lifecycle**

Prepend to `crates/heiwa_workspace/src/worktree.rs`:

```rust
//! Isolated worktrees: where a Work is allowed to change files.
//!
//! A worktree is built from a commit, which is what makes it safe. The user's
//! working tree is never read, written, stashed, or checked out by anything in
//! this module, so uncommitted work cannot be lost by creating one.
//!
//! The naming convention carries the `work_id`, so the owner of a directory is
//! visible from `git worktree list` alone rather than needing a side table
//! that can drift from the filesystem.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::git::git;
use crate::scope::resolve_in_scope;
use crate::WorkspaceError;

const BRANCH_PREFIX: &str = "heiwa/";

/// One Work's isolated checkout of one repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeHandle {
    pub work_id: String,
    /// Canonical path of the worktree directory.
    pub path: String,
    /// The branch the worktree is on, always `heiwa/<work_id>`.
    pub branch: String,
    /// The commit the worktree was created from.
    pub base_commit: String,
}

/// One entry from `git worktree list`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ListedWorktree {
    pub path: String,
    pub head: String,
    pub branch: Option<String>,
    /// Present when the branch follows the Heiwa naming convention.
    pub work_id: Option<String>,
}

fn branch_for(work_id: &str) -> String {
    format!("{BRANCH_PREFIX}{work_id}")
}

/// Create an isolated worktree for `work_id`, under `holding_dir`.
///
/// `holding_dir` is supplied by the caller because this crate does not resolve
/// roots; the shell passes a directory under the runtime root.
pub fn create_worktree_in(
    repo_root: &Path,
    holding_dir: &Path,
    work_id: &str,
) -> Result<WorktreeHandle, WorkspaceError> {
    let branch = branch_for(work_id);

    if list_worktrees_in(repo_root)?
        .iter()
        .any(|listed| listed.work_id.as_deref() == Some(work_id))
    {
        return Err(WorkspaceError::WorktreeExists {
            work_id: work_id.to_string(),
        });
    }

    let target = holding_dir.join(work_id);
    let target = resolve_in_scope(holding_dir, &target)?;

    let base_commit = git(repo_root, &["rev-parse", "HEAD"])?;
    let target_str = target.to_string_lossy().to_string();

    git(
        repo_root,
        &["worktree", "add", "-q", "-b", &branch, &target_str, &base_commit],
    )?;

    Ok(WorktreeHandle {
        work_id: work_id.to_string(),
        path: target_str,
        branch,
        base_commit,
    })
}

/// Every worktree git knows about for this repository.
pub fn list_worktrees_in(repo_root: &Path) -> Result<Vec<ListedWorktree>, WorkspaceError> {
    let raw = git(repo_root, &["worktree", "list", "--porcelain"])?;
    Ok(parse_worktree_list(&raw))
}

/// Remove a worktree and its branch.
pub fn remove_worktree_in(
    repo_root: &Path,
    handle: &WorktreeHandle,
) -> Result<(), WorkspaceError> {
    git(repo_root, &["worktree", "remove", "--force", &handle.path])?;
    // The branch outlives the directory otherwise, and the next Work with the
    // same id would collide with a ref nothing points at.
    let _ = git(repo_root, &["branch", "-D", &handle.branch]);
    Ok(())
}

/// Parse `git worktree list --porcelain`: blank-line separated blocks of
/// `worktree <path>`, `HEAD <sha>`, `branch <ref>`.
fn parse_worktree_list(raw: &str) -> Vec<ListedWorktree> {
    let mut out = Vec::new();
    let mut path: Option<String> = None;
    let mut head = String::new();
    let mut branch: Option<String> = None;

    let mut flush = |path: &mut Option<String>, head: &mut String, branch: &mut Option<String>| {
        if let Some(path_value) = path.take() {
            let work_id = branch
                .as_deref()
                .and_then(|reference| reference.rsplit('/').next().map(str::to_string))
                .filter(|_| {
                    branch
                        .as_deref()
                        .is_some_and(|reference| reference.contains(BRANCH_PREFIX))
                });
            out.push(ListedWorktree {
                path: path_value,
                head: std::mem::take(head),
                branch: branch.take(),
                work_id,
            });
        }
    };

    for line in raw.lines() {
        if line.is_empty() {
            flush(&mut path, &mut head, &mut branch);
        } else if let Some(rest) = line.strip_prefix("worktree ") {
            flush(&mut path, &mut head, &mut branch);
            path = Some(rest.to_string());
        } else if let Some(rest) = line.strip_prefix("HEAD ") {
            head = rest.to_string();
        } else if let Some(rest) = line.strip_prefix("branch ") {
            branch = Some(rest.to_string());
        }
    }
    flush(&mut path, &mut head, &mut branch);
    out
}
```

Add to `WorkspaceError`:

```rust
    #[error("{work_id} already holds a worktree in this repository")]
    WorktreeExists { work_id: String },
```

and to the re-exports:

```rust
pub use worktree::{
    create_worktree_in, list_worktrees_in, remove_worktree_in, ListedWorktree, WorktreeHandle,
};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p heiwa_workspace`
Expected: PASS, 19 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/heiwa_workspace
git commit -m "feat(workspace): give each Work an isolated worktree"
```

---

### Task 5: Exclusive writer lease on the existing evidence stream

**Files:**
- Create: `crates/heiwa_workspace/src/lease.rs`
- Modify: `crates/heiwa_workspace/src/lib.rs`

This task deliberately adds **no** storage. `heiwa_evidence` already has a
`worker_leases` stream, a replay (`WorkerStateView::replay`), and a restart
revocation (`recover_interrupted`). A second lease store would be a second
thing to recover, and one of the two would eventually be forgotten.

- [ ] **Step 1: Write the failing test**

Create `crates/heiwa_workspace/src/lease.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use heiwa_evidence::JsonlTransport;

    fn evidence() -> (tempfile::TempDir, JsonlTransport) {
        let dir = tempfile::tempdir().expect("tempdir");
        let transport = JsonlTransport::new(dir.path().to_path_buf()).expect("transport");
        (dir, transport)
    }

    #[test]
    fn acquiring_a_lease_records_it_as_issued() {
        let (dir, transport) = evidence();
        let lease = acquire_writer_lease(
            dir.path(),
            &transport,
            "work-abc",
            "/repo",
            "install-1",
            "2026-08-24T00:00:00Z",
            "2026-08-24T01:00:00Z",
            || "lease-1".to_string(),
        )
        .expect("acquire");

        assert_eq!(lease.lease_id, "lease-1");
        assert_eq!(lease.work_id, "work-abc");
        assert_eq!(lease.capability, "workspace.write:/repo");
    }

    #[test]
    fn a_second_work_cannot_hold_the_same_repository() {
        let (dir, transport) = evidence();
        acquire_writer_lease(
            dir.path(), &transport, "work-abc", "/repo", "install-1",
            "2026-08-24T00:00:00Z", "2026-08-24T01:00:00Z", || "lease-1".to_string(),
        )
        .expect("first acquire");

        let error = acquire_writer_lease(
            dir.path(), &transport, "work-def", "/repo", "install-1",
            "2026-08-24T00:00:00Z", "2026-08-24T01:00:00Z", || "lease-2".to_string(),
        )
        .expect_err("two writers on one repository is the thing this prevents");

        let WorkspaceError::LeaseHeld { held_by, .. } = &error else {
            panic!("expected LeaseHeld, got {error:?}");
        };
        assert_eq!(held_by, "work-abc", "the refusal must name the holder");
    }

    #[test]
    fn a_different_repository_is_not_blocked_by_an_unrelated_lease() {
        let (dir, transport) = evidence();
        acquire_writer_lease(
            dir.path(), &transport, "work-abc", "/repo-one", "install-1",
            "2026-08-24T00:00:00Z", "2026-08-24T01:00:00Z", || "lease-1".to_string(),
        )
        .expect("first");

        acquire_writer_lease(
            dir.path(), &transport, "work-def", "/repo-two", "install-1",
            "2026-08-24T00:00:00Z", "2026-08-24T01:00:00Z", || "lease-2".to_string(),
        )
        .expect("a lease is per repository, not global");
    }

    #[test]
    fn releasing_a_lease_frees_the_repository() {
        let (dir, transport) = evidence();
        let lease = acquire_writer_lease(
            dir.path(), &transport, "work-abc", "/repo", "install-1",
            "2026-08-24T00:00:00Z", "2026-08-24T01:00:00Z", || "lease-1".to_string(),
        )
        .expect("acquire");

        release_writer_lease(&transport, &lease, "2026-08-24T00:30:00Z").expect("release");

        acquire_writer_lease(
            dir.path(), &transport, "work-def", "/repo", "install-1",
            "2026-08-24T00:31:00Z", "2026-08-24T01:31:00Z", || "lease-2".to_string(),
        )
        .expect("a released repository is available again");
    }

    #[test]
    fn releasing_preserves_the_facts_the_issued_record_carried() {
        // upsert appends and replay keeps the last record, so a release that
        // omits a field silently destroys it.
        let (dir, transport) = evidence();
        let lease = acquire_writer_lease(
            dir.path(), &transport, "work-abc", "/repo", "install-1",
            "2026-08-24T00:00:00Z", "2026-08-24T01:00:00Z", || "lease-1".to_string(),
        )
        .expect("acquire");
        release_writer_lease(&transport, &lease, "2026-08-24T00:30:00Z").expect("release");

        let view = heiwa_evidence::WorkerStateView::replay(dir.path()).expect("replay");
        let stored = view.leases.get("lease-1").expect("lease survives replay");
        assert_eq!(stored.status, "completed");
        assert_eq!(stored.node_id, "install-1", "the node must survive release");
        assert_eq!(
            stored.issued_at, "2026-08-24T00:00:00Z",
            "release must not rewrite when the lease was issued"
        );
    }

    #[test]
    fn a_lease_does_not_survive_a_runtime_restart() {
        // The whole reason for reusing worker_leases: recovery already exists
        // and already does this. A crash must not leave a repository locked
        // forever by a Work that is no longer running.
        let (dir, transport) = evidence();
        acquire_writer_lease(
            dir.path(), &transport, "work-abc", "/repo", "install-1",
            "2026-08-24T00:00:00Z", "2026-08-24T01:00:00Z", || "lease-1".to_string(),
        )
        .expect("acquire");

        let report =
            heiwa_evidence::recover_interrupted(dir.path(), &transport).expect("recover");
        assert_eq!(report.leases_revoked, 1);

        acquire_writer_lease(
            dir.path(), &transport, "work-def", "/repo", "install-1",
            "2026-08-24T02:00:00Z", "2026-08-24T03:00:00Z", || "lease-2".to_string(),
        )
        .expect("restart recovery must free the repository");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Add `pub mod lease;` to `crates/heiwa_workspace/src/lib.rs`.

Run: `cargo test -p heiwa_workspace --lib lease`
Expected: FAIL to compile — `cannot find function 'acquire_writer_lease'`.

- [ ] **Step 3: Write the lease**

Prepend to `crates/heiwa_workspace/src/lease.rs`:

```rust
//! Exclusive permission for one Work to mutate one repository.
//!
//! Deliberately not a new store. `heiwa_evidence` already owns the
//! `worker_leases` stream, its replay, and `recover_interrupted`, which
//! revokes every lease left `issued` or `acked` when the runtime restarts. A
//! lease that outlived a crash would lock a repository against a Work that no
//! longer exists, and reusing the existing recovery is what stops that without
//! a second mechanism to remember.

use std::path::Path;

use heiwa_evidence::{EvidenceTransport, PersistedWorkerLease, WorkerStateView};
use serde::{Deserialize, Serialize};

use crate::WorkspaceError;

/// Statuses that mean a lease is still holding its resource.
const LIVE: [&str; 2] = ["issued", "acked"];

/// One Work's exclusive write hold on one repository.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WriterLease {
    pub lease_id: String,
    pub work_id: String,
    /// `workspace.write:<canonical repository root>` — the resource the lease
    /// is exclusive over. Exclusivity is decided on this string.
    pub capability: String,
    /// Carried so releasing does not erase it. `upsert_worker_lease` appends,
    /// and replay keeps the last record per `lease_id`, so the release record
    /// *replaces* the issued one — anything not repeated here is destroyed.
    pub node_id: String,
    pub issued_at: String,
}

fn capability_for(repo_root: &str) -> String {
    format!("workspace.write:{repo_root}")
}

/// Take the write lease on `repo_root` for `work_id`, or refuse and say who
/// holds it.
///
/// `evidence_dir` is where the journal lives; the caller resolves it, because
/// this crate resolves no roots.
#[allow(clippy::too_many_arguments)]
pub fn acquire_writer_lease<T: EvidenceTransport>(
    evidence_dir: &Path,
    transport: &T,
    work_id: &str,
    repo_root: &str,
    installation_id: &str,
    issued_at: &str,
    expires_at: &str,
    new_lease_id: impl FnOnce() -> String,
) -> Result<WriterLease, WorkspaceError> {
    let capability = capability_for(repo_root);

    let view = WorkerStateView::replay(evidence_dir)
        .map_err(|error| WorkspaceError::Evidence(error.to_string()))?;

    if let Some(held) = view
        .leases
        .values()
        .find(|lease| lease.capability == capability && LIVE.contains(&lease.status.as_str()))
    {
        return Err(WorkspaceError::LeaseHeld {
            repo_root: repo_root.to_string(),
            held_by: held.task_id.clone(),
        });
    }

    let lease_id = new_lease_id();
    transport
        .upsert_worker_lease(PersistedWorkerLease {
            lease_id: lease_id.clone(),
            task_id: work_id.to_string(),
            // A1-b has no separate worker session yet; A1-c introduces one and
            // will carry it here. Naming the Work is honest in the meantime.
            session_id: work_id.to_string(),
            // No mesh node identity is required for local Work, exactly as
            // `Work.origin_node` stays `None` until enrolment.
            node_id: installation_id.to_string(),
            capability: capability.clone(),
            status: "issued".to_string(),
            issued_at: issued_at.to_string(),
            updated_at: issued_at.to_string(),
            expires_at: expires_at.to_string(),
            acked_at: None,
            completed_at: None,
            failure_code: None,
            reason: None,
        })
        .map_err(|error| WorkspaceError::Evidence(error.to_string()))?;

    Ok(WriterLease {
        lease_id,
        work_id: work_id.to_string(),
        capability,
        node_id: installation_id.to_string(),
        issued_at: issued_at.to_string(),
    })
}

/// Give the repository back.
pub fn release_writer_lease<T: EvidenceTransport>(
    transport: &T,
    lease: &WriterLease,
    released_at: &str,
) -> Result<(), WorkspaceError> {
    transport
        .upsert_worker_lease(PersistedWorkerLease {
            lease_id: lease.lease_id.clone(),
            task_id: lease.work_id.clone(),
            session_id: lease.work_id.clone(),
            node_id: lease.node_id.clone(),
            capability: lease.capability.clone(),
            status: "completed".to_string(),
            issued_at: lease.issued_at.clone(),
            updated_at: released_at.to_string(),
            expires_at: released_at.to_string(),
            acked_at: None,
            completed_at: Some(released_at.to_string()),
            failure_code: None,
            reason: Some("workspace released".to_string()),
        })
        .map_err(|error| WorkspaceError::Evidence(error.to_string()))
}
```

Add to `WorkspaceError`:

```rust
    #[error("{repo_root} is already held for writing by {held_by}")]
    LeaseHeld { repo_root: String, held_by: String },
    #[error("evidence journal error: {0}")]
    Evidence(String),
```

and to the re-exports:

```rust
pub use lease::{acquire_writer_lease, release_writer_lease, WriterLease};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p heiwa_workspace`
Expected: PASS, 25 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/heiwa_workspace
git commit -m "feat(workspace): hold one repository per Work through the existing lease stream"
```

---

### Task 6: The refusal boundary around uncommitted work

Preservation is free — `git worktree add` builds from a commit. What is not
free is refusing the commands that destroy uncommitted work. This task makes
that refusal explicit, tested, and hard to remove by accident.

**Files:**
- Modify: `crates/heiwa_workspace/src/git.rs`
- Test: `crates/heiwa_workspace/src/git.rs`

- [ ] **Step 1: Write the failing test**

Add to the test module in `crates/heiwa_workspace/src/git.rs`:

```rust
    #[test]
    fn commands_that_discard_uncommitted_work_are_refused() {
        let dir = repo();
        std::fs::write(dir.path().join("a.txt"), "one\nMINE\n").expect("dirty it");

        for destructive in [
            vec!["reset", "--hard"],
            vec!["checkout", "-f", "main"],
            vec!["stash"],
            vec!["stash", "push"],
            vec!["clean", "-fd"],
            vec!["restore", "a.txt"],
        ] {
            let error = git(dir.path(), &destructive)
                .expect_err("this command can destroy the user's work");
            assert!(
                matches!(error, GitError::Refused { .. }),
                "{destructive:?} must be refused, got {error:?}"
            );
        }

        let survived = std::fs::read_to_string(dir.path().join("a.txt")).expect("read");
        assert_eq!(survived, "one\nMINE\n", "nothing may have run");
    }

    #[test]
    fn a_refusal_names_the_command_it_refused() {
        let dir = repo();
        let error = git(dir.path(), &["reset", "--hard"]).expect_err("refused");
        let GitError::Refused { args, .. } = &error else {
            panic!("expected Refused, got {error:?}");
        };
        assert!(args.contains("reset"), "{args}");
    }

    #[test]
    fn reading_and_worktree_commands_are_still_allowed() {
        let dir = repo();
        git(dir.path(), &["status", "--porcelain=v1"]).expect("status is a read");
        git(dir.path(), &["rev-parse", "HEAD"]).expect("rev-parse is a read");
        // `checkout` without -f is how a worktree is made; only the forced
        // form discards work.
        git(dir.path(), &["worktree", "list"]).expect("worktree list is a read");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p heiwa_workspace --lib git`
Expected: FAIL — `no variant named 'Refused'`.

- [ ] **Step 3: Write the refusal**

Add to `GitError` in `crates/heiwa_workspace/src/git.rs`:

```rust
    #[error("refused to run `git {args}` in {dir}: it can discard uncommitted work")]
    Refused { dir: String, args: String },
```

Add above `pub fn git`:

```rust
/// Commands that can destroy work the user has not committed.
///
/// Heiwa mutates inside its own worktrees, which are built from commits, so it
/// never needs any of these. Refusing them here — at the single process
/// boundary — means no future call site can reach them by accident, and the
/// list is one place to review rather than a convention to remember.
fn discards_uncommitted_work(args: &[&str]) -> bool {
    match args {
        ["reset", rest @ ..] => rest.contains(&"--hard"),
        ["checkout", rest @ ..] => rest.contains(&"-f") || rest.contains(&"--force"),
        ["stash", ..] => true,
        ["clean", ..] => true,
        ["restore", ..] => true,
        _ => false,
    }
}
```

and as the first statement inside `pub fn git`:

```rust
    if discards_uncommitted_work(args) {
        return Err(GitError::Refused {
            dir: dir.display().to_string(),
            args: args.join(" "),
        });
    }
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p heiwa_workspace`
Expected: PASS, 28 tests.

Note: `remove_worktree_in` passes `--force` to `worktree remove`, not to
`checkout`, so it is unaffected. If it started failing, the guard is matching
too broadly — fix the guard, do not weaken the refusal.

- [ ] **Step 5: Commit**

```bash
git add crates/heiwa_workspace
git commit -m "feat(workspace): refuse git commands that discard uncommitted work"
```

---

### Task 7: Bounded diff projection

**Files:**
- Create: `crates/heiwa_workspace/src/projection.rs`
- Modify: `crates/heiwa_workspace/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/heiwa_workspace/src/projection.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::tests::repo;
    use crate::worktree::create_worktree_in;

    fn worktree_with_changes(files: usize) -> (tempfile::TempDir, tempfile::TempDir, String) {
        let source = repo();
        let holding = tempfile::tempdir().expect("holding");
        let handle =
            create_worktree_in(source.path(), holding.path(), "work-abc").expect("worktree");
        for index in 0..files {
            std::fs::write(
                std::path::Path::new(&handle.path).join(format!("f{index}.txt")),
                "content\n",
            )
            .expect("write");
        }
        let path = handle.path.clone();
        (source, holding, path)
    }

    #[test]
    fn an_unchanged_worktree_projects_an_empty_diff() {
        let (_source, _holding, path) = worktree_with_changes(0);
        let diff = diff_projection_in(std::path::Path::new(&path), 100).expect("diff");

        assert!(diff.files.is_empty());
        assert_eq!(diff.total_files, 0);
        assert!(!diff.truncated);
    }

    #[test]
    fn a_changed_file_appears_with_its_line_counts() {
        let (_source, _holding, path) = worktree_with_changes(0);
        std::fs::write(std::path::Path::new(&path).join("a.txt"), "one\ntwo\n").expect("edit");

        let diff = diff_projection_in(std::path::Path::new(&path), 100).expect("diff");
        assert_eq!(diff.total_files, 1);
        let file = &diff.files[0];
        assert_eq!(file.path, "a.txt");
        assert_eq!(file.added, 1, "one line added");
        assert_eq!(file.removed, 0);
    }

    #[test]
    fn a_new_file_is_included_even_though_git_does_not_track_it_yet() {
        // An untracked file is a real change the user must see before
        // approving anything. A plain `git diff` would hide it entirely.
        let (_source, _holding, path) = worktree_with_changes(1);
        let diff = diff_projection_in(std::path::Path::new(&path), 100).expect("diff");

        assert_eq!(diff.total_files, 1);
        assert_eq!(diff.files[0].path, "f0.txt");
    }

    #[test]
    fn a_diff_larger_than_the_limit_is_truncated_and_says_so() {
        let (_source, _holding, path) = worktree_with_changes(5);
        let diff = diff_projection_in(std::path::Path::new(&path), 2).expect("diff");

        assert_eq!(diff.files.len(), 2, "only the limit is carried");
        assert_eq!(diff.total_files, 5, "the true count is still reported");
        assert!(
            diff.truncated,
            "a silently shortened diff would be read as the whole change"
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Add `pub mod projection;` to `crates/heiwa_workspace/src/lib.rs`.

Run: `cargo test -p heiwa_workspace --lib projection`
Expected: FAIL to compile — `cannot find function 'diff_projection_in'`.

- [ ] **Step 3: Write the diff projection**

Prepend to `crates/heiwa_workspace/src/projection.rs`:

```rust
//! What changed, bounded.
//!
//! A projection is delivered to a client, so it is capped by construction. The
//! cap is reported alongside the true total: a shortened diff that does not
//! say it was shortened reads as the whole change, and someone approves it.
//!
//! Full file contents are not carried here. They load through their own
//! request, exactly as the A1-a snapshot contract requires.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::git::git;
use crate::WorkspaceError;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedFile {
    pub path: String,
    pub added: u64,
    pub removed: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffProjectionV1 {
    /// At most `limit` entries.
    pub files: Vec<ChangedFile>,
    /// How many files actually changed, before the cap.
    pub total_files: usize,
    pub truncated: bool,
}

/// Project the changes in `worktree`, including files git does not track yet.
pub fn diff_projection_in(
    worktree: &Path,
    limit: usize,
) -> Result<DiffProjectionV1, WorkspaceError> {
    // Staged-and-unstaged tracked changes, with counts.
    let numstat = git(worktree, &["diff", "--numstat", "HEAD"])?;
    let mut files: Vec<ChangedFile> = numstat
        .lines()
        .filter_map(parse_numstat_line)
        .collect();

    // Untracked files are changes too. `git diff` omits them, and a reviewer
    // who cannot see a newly added file is reviewing the wrong change.
    let untracked = git(
        worktree,
        &["ls-files", "--others", "--exclude-standard"],
    )?;
    for path in untracked.lines().filter(|line| !line.trim().is_empty()) {
        let added = std::fs::read_to_string(worktree.join(path))
            .map(|body| body.lines().count() as u64)
            .unwrap_or(0);
        files.push(ChangedFile {
            path: path.to_string(),
            added,
            removed: 0,
        });
    }

    files.sort_by(|left, right| left.path.cmp(&right.path));

    let total_files = files.len();
    let truncated = total_files > limit;
    files.truncate(limit);

    Ok(DiffProjectionV1 {
        files,
        total_files,
        truncated,
    })
}

/// `git diff --numstat` lines are `<added>\t<removed>\t<path>`. A binary file
/// reports `-` for both counts, which becomes zero rather than an error.
fn parse_numstat_line(line: &str) -> Option<ChangedFile> {
    let mut parts = line.splitn(3, '\t');
    let added = parts.next()?;
    let removed = parts.next()?;
    let path = parts.next()?.trim();
    if path.is_empty() {
        return None;
    }
    Some(ChangedFile {
        path: path.to_string(),
        added: added.parse().unwrap_or(0),
        removed: removed.parse().unwrap_or(0),
    })
}
```

Add to the re-exports in `crates/heiwa_workspace/src/lib.rs`:

```rust
pub use projection::{diff_projection_in, ChangedFile, DiffProjectionV1};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p heiwa_workspace`
Expected: PASS, 32 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/heiwa_workspace
git commit -m "feat(workspace): project what changed, bounded and honest about it"
```

---

### Task 8: Test projection

**Files:**
- Modify: `crates/heiwa_workspace/src/projection.rs`

- [ ] **Step 1: Write the failing test**

Add to the test module in `crates/heiwa_workspace/src/projection.rs`:

```rust
    #[test]
    fn a_passing_command_projects_a_pass_with_its_output() {
        let (_source, _holding, path) = worktree_with_changes(0);
        let result = test_projection_in(
            std::path::Path::new(&path),
            "echo",
            &["all good"],
            4096,
        )
        .expect("run");

        assert!(result.passed);
        assert_eq!(result.exit_code, Some(0));
        assert!(result.output.contains("all good"), "{}", result.output);
        assert!(!result.truncated);
    }

    #[test]
    fn a_failing_command_projects_a_failure_rather_than_an_error() {
        // A failing test suite is a fact about the code, not a fault in
        // Heiwa. It has to survive into the projection so review can see it.
        let (_source, _holding, path) = worktree_with_changes(0);
        let result =
            test_projection_in(std::path::Path::new(&path), "false", &[], 4096).expect("run");

        assert!(!result.passed);
        assert_ne!(result.exit_code, Some(0));
    }

    #[test]
    fn long_output_keeps_the_tail_and_says_it_was_cut() {
        // The failure is at the end of a test run, so the tail is the part
        // worth keeping.
        let (_source, _holding, path) = worktree_with_changes(0);
        let long = "x".repeat(500);
        let result =
            test_projection_in(std::path::Path::new(&path), "echo", &[&long], 100).expect("run");

        assert!(result.truncated);
        assert!(result.output.len() <= 100, "len {}", result.output.len());
        assert!(result.output.ends_with('x'), "the tail must be what survives");
    }

    #[test]
    fn a_command_that_does_not_exist_is_an_error_not_a_failed_test() {
        let (_source, _holding, path) = worktree_with_changes(0);
        let error = test_projection_in(
            std::path::Path::new(&path),
            "heiwa-no-such-binary",
            &[],
            4096,
        )
        .expect_err("a missing runner is not a test result");
        assert!(matches!(error, WorkspaceError::CommandUnavailable { .. }), "{error:?}");
    }
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p heiwa_workspace --lib projection`
Expected: FAIL to compile — `cannot find function 'test_projection_in'`.

- [ ] **Step 3: Write the test projection**

Append to `crates/heiwa_workspace/src/projection.rs`, above the test module:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestProjectionV1 {
    pub command: String,
    pub passed: bool,
    /// `None` when the process was killed by a signal.
    pub exit_code: Option<i32>,
    /// The tail of combined stdout and stderr, capped.
    pub output: String,
    pub truncated: bool,
}

/// Run a verification command inside `worktree` and record the outcome.
///
/// This is the crate's second and last process boundary, kept separate from
/// `git()` on purpose: this one runs whatever the user configured as their
/// verification, so it must never inherit git's allowances or its refusals.
///
/// A non-zero exit is a **result**, not an error. A missing binary is an
/// error, because no test actually ran and reporting "failed" would be a lie.
pub fn test_projection_in(
    worktree: &Path,
    command: &str,
    args: &[&str],
    output_limit: usize,
) -> Result<TestProjectionV1, WorkspaceError> {
    let output = std::process::Command::new(command)
        .args(args)
        .current_dir(worktree)
        .output()
        .map_err(|error| WorkspaceError::CommandUnavailable {
            command: command.to_string(),
            reason: error.to_string(),
        })?;

    let mut combined = String::from_utf8_lossy(&output.stdout).to_string();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    let combined = combined.trim().to_string();

    let truncated = combined.len() > output_limit;
    let output_text = if truncated {
        // Keep the tail: a run's failure is at the end, not the beginning.
        // Slice on a char boundary so multi-byte output cannot panic here.
        let start = combined.len() - output_limit;
        let start = (start..combined.len())
            .find(|index| combined.is_char_boundary(*index))
            .unwrap_or(combined.len());
        combined[start..].to_string()
    } else {
        combined
    };

    Ok(TestProjectionV1 {
        command: std::iter::once(command)
            .chain(args.iter().copied())
            .collect::<Vec<_>>()
            .join(" "),
        passed: output.status.success(),
        exit_code: output.status.code(),
        output: output_text,
        truncated,
    })
}
```

Add to `WorkspaceError`:

```rust
    // Not named `source`: thiserror reserves that for a nested Error, and
    // this is the io failure rendered as text.
    #[error("could not run verification command {command}: {reason}")]
    CommandUnavailable { command: String, reason: String },
```

and extend the projection re-export:

```rust
pub use projection::{
    diff_projection_in, test_projection_in, ChangedFile, DiffProjectionV1, TestProjectionV1,
};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p heiwa_workspace`
Expected: PASS, 36 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/heiwa_workspace
git commit -m "feat(workspace): project verification results, keeping the tail"
```

---

### Task 9: Workspace facts reach Work as scoped operator events

**Files:**
- Modify: `crates/heiwa_evidence/src/operator.rs`
- Modify: `crates/heiwa_session/src/operator.rs`
- Create: `crates/heiwa_workspace/src/events.rs`
- Modify: `crates/heiwa_workspace/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/heiwa_workspace/src/events.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn handle() -> crate::worktree::WorktreeHandle {
        crate::worktree::WorktreeHandle {
            work_id: "work-abc".to_string(),
            path: "/holding/work-abc".to_string(),
            branch: "heiwa/work-abc".to_string(),
            base_commit: "abc123".to_string(),
        }
    }

    #[test]
    fn a_prepared_event_names_its_work_and_carries_the_worktree() {
        let event = workspace_prepared_event(
            "work-abc",
            "thread-1",
            "/repo",
            &handle(),
            "2026-08-24T00:00:00Z",
            || "evt-1".to_string(),
        );

        assert_eq!(event.work_id.as_deref(), Some("work-abc"));
        assert_eq!(event.event_type, OperatorEventType::WorkspacePrepared);

        let payload = WorkspacePreparedPayload::from_event(&event).expect("payload");
        assert_eq!(payload.repo_root, "/repo");
        assert_eq!(payload.worktree_path, "/holding/work-abc");
        assert_eq!(payload.base_commit, "abc123");
    }

    #[test]
    fn a_released_event_names_the_work_it_freed() {
        let event = workspace_released_event(
            "work-abc",
            "thread-1",
            "/repo",
            "2026-08-24T00:10:00Z",
            || "evt-2".to_string(),
        );

        assert_eq!(event.event_type, OperatorEventType::WorkspaceReleased);
        let payload = WorkspaceReleasedPayload::from_event(&event).expect("payload");
        assert_eq!(payload.repo_root, "/repo");
    }

    #[test]
    fn a_payload_from_the_wrong_event_type_is_refused() {
        let prepared = workspace_prepared_event(
            "work-abc", "thread-1", "/repo", &handle(),
            "2026-08-24T00:00:00Z", || "evt-1".to_string(),
        );
        assert!(WorkspaceReleasedPayload::from_event(&prepared).is_none());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Add `pub mod events;` to `crates/heiwa_workspace/src/lib.rs`.

Run: `cargo test -p heiwa_workspace --lib events`
Expected: FAIL to compile — `no variant named 'WorkspacePrepared'`.

- [ ] **Step 3: Add the event types and the scope rule**

In `crates/heiwa_evidence/src/operator.rs`, add to `OperatorEventType` after
`WorkLinked`:

```rust
    WorkspacePrepared,
    WorkspaceReleased,
```

In `crates/heiwa_session/src/operator.rs`, extend `requires_work_id` — a
workspace event with no Work is a hold belonging to nobody:

```rust
fn requires_work_id(event_type: &OperatorEventType) -> bool {
    matches!(
        event_type,
        OperatorEventType::WorkCreated
            | OperatorEventType::WorkLinked
            | OperatorEventType::WorkspacePrepared
            | OperatorEventType::WorkspaceReleased
    )
}
```

The closed enum will now break `apply_to_existing_thread` at
`crates/heiwa_session/src/operator.rs:1521`. That is the enum doing its job —
A1-a hit the same wall. Workspace events name a thread without opening or
closing a turn, so extend the same non-terminal arm:

```rust
        | OperatorEventType::WorkCreated
        | OperatorEventType::WorkLinked
        | OperatorEventType::WorkspacePrepared
        | OperatorEventType::WorkspaceReleased => apply_nonterminal_touch(entry, event),
```

- [ ] **Step 4: Write the builders**

Prepend to `crates/heiwa_workspace/src/events.rs`:

```rust
//! Workspace facts as operator-domain events.
//!
//! `Work` is a fold over the operator journal (A1-a). Workspace facts join the
//! same stream rather than a second store, so replaying one Work produces the
//! whole of it — including which repository it held and where it was allowed
//! to write.

use heiwa_evidence::{
    OperatorActor, OperatorEvent, OperatorEventType, OperatorRisk, OperatorSensitivity,
    OPERATOR_EVENT_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};

use crate::worktree::WorktreeHandle;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspacePreparedPayload {
    pub repo_root: String,
    pub worktree_path: String,
    pub branch: String,
    pub base_commit: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceReleasedPayload {
    pub repo_root: String,
}

impl WorkspacePreparedPayload {
    pub fn from_event(event: &OperatorEvent) -> Option<Self> {
        if event.event_type != OperatorEventType::WorkspacePrepared {
            return None;
        }
        serde_json::from_value(event.payload.clone()).ok()
    }
}

impl WorkspaceReleasedPayload {
    pub fn from_event(event: &OperatorEvent) -> Option<Self> {
        if event.event_type != OperatorEventType::WorkspaceReleased {
            return None;
        }
        serde_json::from_value(event.payload.clone()).ok()
    }
}

fn scoped(
    work_id: &str,
    thread_id: &str,
    event_type: OperatorEventType,
    occurred_at: &str,
    payload: serde_json::Value,
    new_event_id: impl FnOnce() -> String,
) -> OperatorEvent {
    OperatorEvent {
        schema_version: OPERATOR_EVENT_SCHEMA_VERSION,
        event_id: new_event_id(),
        thread_id: thread_id.to_string(),
        turn_id: None,
        run_id: None,
        call_id: None,
        work_id: Some(work_id.to_string()),
        event_type,
        occurred_at: occurred_at.to_string(),
        actor: OperatorActor {
            kind: "user".to_string(),
            id: "local".to_string(),
        },
        risk_class: OperatorRisk::Low,
        sensitivity: OperatorSensitivity::LocalPrivate,
        parent_event_id: None,
        correlation_id: None,
        source_refs: Vec::new(),
        evidence_refs: Vec::new(),
        payload,
    }
}

pub fn workspace_prepared_event(
    work_id: &str,
    thread_id: &str,
    repo_root: &str,
    handle: &WorktreeHandle,
    occurred_at: &str,
    new_event_id: impl FnOnce() -> String,
) -> OperatorEvent {
    let payload = serde_json::json!({
        "repo_root": repo_root,
        "worktree_path": handle.path,
        "branch": handle.branch,
        "base_commit": handle.base_commit,
    });
    scoped(
        work_id,
        thread_id,
        OperatorEventType::WorkspacePrepared,
        occurred_at,
        payload,
        new_event_id,
    )
}

pub fn workspace_released_event(
    work_id: &str,
    thread_id: &str,
    repo_root: &str,
    occurred_at: &str,
    new_event_id: impl FnOnce() -> String,
) -> OperatorEvent {
    let payload = serde_json::json!({ "repo_root": repo_root });
    scoped(
        work_id,
        thread_id,
        OperatorEventType::WorkspaceReleased,
        occurred_at,
        payload,
        new_event_id,
    )
}
```

Add to the re-exports:

```rust
pub use events::{
    workspace_prepared_event, workspace_released_event, WorkspacePreparedPayload,
    WorkspaceReleasedPayload,
};
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p heiwa_workspace -p heiwa_evidence -p heiwa-session`
Expected: PASS, including the 3 new event tests.

- [ ] **Step 6: Commit**

```bash
git add crates/heiwa_evidence crates/heiwa_session crates/heiwa_workspace
git commit -m "feat(workspace): record workspace facts as work-scoped operator events"
```

---

### Task 10: Integration through the real journal and a real repository

**Files:**
- Create: `crates/heiwa_workspace/tests/workspace_core.rs`
- Modify: `crates/heiwa_workspace/Cargo.toml` (dev-dependencies)

- [ ] **Step 1: Write the test**

Add to `[dev-dependencies]` in `crates/heiwa_workspace/Cargo.toml`:

```toml
heiwa-session = { path = "../heiwa_session" }
```

Same reasoning as A1-a Task 8: the edge points upward from a foundation crate
to a runtime one, which Cargo permits for dev-dependencies only. Do not promote
it.

Create `crates/heiwa_workspace/tests/workspace_core.rs`:

```rust
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
            &work_id, "thread-1", "prepare the release", "install-1",
            "2026-08-24T00:00:00Z", || "evt-1".to_string(),
        ))
        .expect("work_created");

    let handle = create_worktree_in(source.path(), holding.path(), work_id.as_str())
        .expect("worktree");
    let snapshot = snapshot_in(source.path()).expect("snapshot");

    service
        .append_event(workspace_prepared_event(
            work_id.as_str(), "thread-1", &snapshot.root, &handle,
            "2026-08-24T00:01:00Z", || "evt-2".to_string(),
        ))
        .expect("workspace_prepared");

    let page = service.events_after("thread-1", None, 64).expect("replay");
    let events: Vec<_> = page.events.into_iter().map(|row| row.event).collect();
    let projection = fold(&events);
    let work = projection.work(work_id.as_str()).expect("work replays");

    assert_eq!(work.revision, 1, "workspace events do not advance the Work revision yet");
    assert_eq!(projection.skipped_events, 0, "a scoped workspace event is not damage");
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

    let handle =
        create_worktree_in(source.path(), holding.path(), "work-abc").expect("worktree");
    let mut event = workspace_prepared_event(
        "work-abc", "thread-1", "/repo", &handle,
        "2026-08-24T00:01:00Z", || "evt-1".to_string(),
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
        evidence.path(), &transport, "work-abc", &snapshot.root, "install-1",
        "2026-08-24T00:00:00Z", "2026-08-24T01:00:00Z", || "lease-1".to_string(),
    )
    .expect("acquire");

    let handle =
        create_worktree_in(source.path(), holding.path(), "work-abc").expect("worktree");
    std::fs::write(std::path::Path::new(&handle.path).join("a.txt"), "one\ntwo\n")
        .expect("edit inside the worktree");

    let diff = diff_projection_in(std::path::Path::new(&handle.path), 50).expect("diff");
    assert_eq!(diff.total_files, 1);
    assert_eq!(diff.files[0].path, "a.txt");

    // The user's own working tree never moved.
    let untouched = std::fs::read_to_string(source.path().join("a.txt")).expect("read");
    assert_eq!(untouched, "one\n");

    release_writer_lease(&transport, &lease, "2026-08-24T00:30:00Z").expect("release");
    acquire_writer_lease(
        evidence.path(), &transport, "work-def", &snapshot.root, "install-1",
        "2026-08-24T00:31:00Z", "2026-08-24T01:31:00Z", || "lease-2".to_string(),
    )
    .expect("the repository is free again");

    let _ = workspace_released_event(
        "work-abc", "thread-1", &snapshot.root, "2026-08-24T00:30:00Z",
        || "evt-3".to_string(),
    );
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test -p heiwa_workspace --test workspace_core`
Expected: PASS, 3 tests. No new production code should be needed; a failure
here means an earlier task's contract is wrong, not this one.

- [ ] **Step 3: Commit**

```bash
git add crates/heiwa_workspace Cargo.lock
git commit -m "test(workspace): prove the hold composes through journal and repository"
```

---

### Task 11: `heiwa workspace` command

**Files:**
- Create: `apps/heiwa_shell/src/cmd/workspace.rs`
- Modify: `apps/heiwa_shell/Cargo.toml`
- Modify: `apps/heiwa_shell/src/cmd/mod.rs`
- Modify: `apps/heiwa_shell/src/cli.rs`
- Modify: `apps/heiwa_shell/src/main.rs`

- [ ] **Step 1: Write the failing test**

Create `apps/heiwa_shell/src/cmd/workspace.rs` with only this test module:

```rust
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
        let prepared = prepare_for(runtime.path(), source.path(), "work-abc", "install-1")
            .expect("prepare");

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
```

- [ ] **Step 2: Run test to verify it fails**

Add to `apps/heiwa_shell/Cargo.toml`, after the `heiwa_work` line:

```toml
heiwa_workspace = { path = "../../crates/heiwa_workspace" }
```

Add `pub mod workspace;` to `apps/heiwa_shell/src/cmd/mod.rs` after `pub mod work;`.

Run: `cargo test -p heiwa-shell --bin heiwa cmd::workspace`
Expected: FAIL to compile — `cannot find function 'status_for'`.

- [ ] **Step 3: Write the command**

Prepend to `apps/heiwa_shell/src/cmd/workspace.rs`:

```rust
//! `heiwa workspace` — what a Work is allowed to touch on disk.
//!
//! Every mutation goes through `heiwa_workspace`, which refuses the git
//! commands that discard uncommitted work. This module resolves the runtime
//! root once and passes paths down; it holds no policy of its own.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};

use heiwa_evidence::JsonlTransport;
use heiwa_workspace::{
    acquire_writer_lease, create_worktree_in, diff_projection_in, snapshot_in,
};

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
    println!("  heiwa workspace prepare <work_id> [--json]      take an isolated worktree and the write lease");
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
    let dirty = status["dirty_paths"].as_array().cloned().unwrap_or_default();
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
        println!("{} holds {}", work_id, prepared["repo_root"].as_str().unwrap_or("?"));
        println!("  worktree {}", prepared["worktree_path"].as_str().unwrap_or("?"));
        println!("  branch   {}", prepared["branch"].as_str().unwrap_or("?"));
        println!("  base     {}", prepared["base_commit"].as_str().unwrap_or("?"));
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
    let handle = create_worktree_in(repo_root, &holding, work_id)
        .map_err(|error| anyhow!("{error}"))?;

    let diff = diff_projection_in(Path::new(&handle.path), 200)
        .map_err(|error| anyhow!("{error}"))?;

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
```

Register the command in `apps/heiwa_shell/src/cli.rs`, immediately before the
`Some("mail")` arm:

```rust
        Some("workspace") => {
            cmd::workspace::run(&args[2..])?;
            Ok(true)
        }
```

Add to `print_help()` in `apps/heiwa_shell/src/main.rs`, after the `work` line:

```rust
    println!("  workspace status|prepare      Repository hold for a Work");
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p heiwa-shell --bin heiwa cmd::workspace`
Expected: PASS, 4 tests.

- [ ] **Step 5: Smoke test the real binary**

```bash
cargo build -p heiwa-shell --bin heiwa
./target/debug/heiwa workspace status
```

Expected: this repository's root, branch `dev`, its HEAD, and either `clean` or
the exact uncommitted paths. Confirm `git status` agrees.

- [ ] **Step 6: Commit**

```bash
git add apps/heiwa_shell Cargo.lock
git commit -m "feat(shell): add heiwa workspace for repository holds"
```

---

### Task 12: CI plumbing, ledger, and the full gate run

**Files:**
- Modify: `scripts/ci_rust_test_group.sh`
- Modify: `docs/superpowers/ledgers/2026-08-22-work-fabric-task-ledger.md`

- [ ] **Step 1: Register the crate with CI**

In `scripts/ci_rust_test_group.sh`, add `heiwa_workspace` to
`foundation_packages` after `heiwa_work`, and `workspace_core` to
`foundation_b_targets` after `work_core`. Keep both lists alphabetical.

- [ ] **Step 2: Verify the grouping matches Cargo**

Run: `bash scripts/ci_rust_test_group.sh --check`
Expected: `Rust CI test groups cover every non-desktop workspace package exactly once.`

This validator is the reason a new crate cannot silently compile in CI without
its tests ever running.

- [ ] **Step 3: Update the ledger**

In `docs/superpowers/ledgers/2026-08-22-work-fabric-task-ledger.md`, add this
section after the A1-a table, and remove A1-b from **Not started**:

```markdown
## Release A1-b — Workspace Coordinator

| # | Step | Status | Verification |
|---|---|---|---|
| 1 | Single git process boundary | todo | `cargo test -p heiwa_workspace` |
| 2 | Repository snapshot | todo | `cargo test -p heiwa_workspace` |
| 3 | Canonical roots and symlink refusal | todo | `cargo test -p heiwa_workspace` |
| 4 | Isolated worktree lifecycle | todo | `cargo test -p heiwa_workspace` |
| 5 | Writer lease on the evidence stream | todo | `cargo test -p heiwa_workspace` |
| 6 | Refusal boundary for uncommitted work | todo | `cargo test -p heiwa_workspace` |
| 7 | Bounded diff projection | todo | `cargo test -p heiwa_workspace` |
| 8 | Test projection | todo | `cargo test -p heiwa_workspace` |
| 9 | Workspace operator events | todo | `cargo test -p heiwa_workspace -p heiwa-session` |
| 10 | Integration through journal and repository | todo | `cargo test -p heiwa_workspace --test workspace_core` |
| 11 | `heiwa workspace` command | todo | `cargo test -p heiwa-shell --bin heiwa cmd::workspace` |
| 12 | CI grouping and ledger | todo | `bash scripts/ci_rust_test_group.sh --check` |
```

- [ ] **Step 4: Run every gate CI runs**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --exclude heiwa-desktop --locked --all-targets -- -D warnings
cargo machete
cargo test --workspace --exclude heiwa-desktop --locked --no-default-features
bash scripts/check_ci_local.sh
```

Expected: `ALL GREEN — safe to push.`

Two lessons from A1-a, both of which cost a cycle there:

- `cargo machete` is part of `check_ci_local.sh`. A dependency listed in the
  manifest but never called fails it. If `heiwa_workspace` ends up not calling
  `serde_json` directly, drop it rather than adding a machete ignore.
- `check_agent_baseline` fails on a dirty tree or an untracked file. Commit
  first, then run it.

- [ ] **Step 5: Refresh the layer stamps and commit**

```bash
bash scripts/check_l0_acceptance.sh
bash scripts/check_l1_acceptance.sh
bash scripts/check_l2_acceptance.sh
git add scripts/ci_rust_test_group.sh docs/superpowers/ledgers
git commit -m "chore(workspace): register heiwa_workspace with CI and log A1-b"
```

Stamps bind to an exact clean HEAD, so run them after the final commit. L0 in
particular will fail if anything in `heiwa_workspace` resolved a home or state
root — it must not.

---

## Deferred with reason

- **Multi-repository anything.** `WorkTaskGraphV1`, scope reservation,
  overlapping-write refusal, barriers, and publication sagas are Release A2.
  A1-b's lease is per repository and that is the whole of it.
- **`SandboxMode::Worktree` wiring.** The variant exists with no consumers.
  It belongs to the worker contract in A1-c, where something actually runs
  inside the worktree.
- **Commit and push from a worktree.** External writes need the Action Gate,
  which is A1-c. A1-b takes the hold and shows the diff; it does not publish.
- **Upstream divergence.** `RepositorySnapshotV1` has no ahead/behind counts.
  They need a remote and a fetch, which is a network effect that belongs with
  the GitHub Collaboration Service in Release B.

## Not started

- A1-c — worker and pane bound to Work, tri-surface agreement, approval to
  receipt through the Action Gate, restart recovery, and
  `scripts/check_work_fabric_a1_acceptance.sh`.

## Definition of Done for A1-b

- `heiwa workspace status` reports this repository's branch, HEAD, and exact
  uncommitted paths, and changes nothing.
- `heiwa workspace prepare <work_id>` takes the write lease and an isolated
  worktree, and reports where it is.
- A second Work preparing the same repository is refused **by name** — the
  refusal says who holds it.
- A worktree is created from a commit, and a modified file in the user's
  working tree is byte-identical afterwards.
- `git reset --hard`, `checkout -f`, `stash`, `clean`, and `restore` are
  refused at the process boundary, with the refused command named.
- A path that resolves outside its root — including through a symlink — is
  refused.
- A lease does not survive `recover_interrupted`; the repository is free again
  after a simulated restart.
- A diff over the limit reports `truncated: true` and the true `total_files`.
- A failing verification command projects `passed: false`, while a missing
  binary is an error rather than a false failure.
- A workspace event without a `work_id` is refused by the writer.
- `bash scripts/check_ci_local.sh` is green and every ledger row is `done`
  with its verification command recorded.
