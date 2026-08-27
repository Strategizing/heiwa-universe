# Work Fabric A1-c2 — Worker and Pane Identity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Launch one provider-owned worker process inside the worktree A1-b prepared for a durable `Work`, bind one durable terminal pane to that same Work and worker identity, and surface both as `runs` rows in the canonical Work-session projection.

**Architecture:** A new I/O-free crate `heiwa_worker` owns worker and pane identity, their state machines, and a pure fold over the operator stream. `apps/heiwa_shell` owns the only process spawn: it resolves the prepared worktree, verifies the executable, appends `worker_launched` through `OperatorSessionService` *before* spawning, streams the child's output into a bounded pane projection, and appends `worker_exited` on reap. The A1-b writer lease stops naming the Work in its `session_id` field and starts naming the worker, closing the seam `crates/heiwa_workspace/src/lease.rs` already documents.

**Tech Stack:** Rust 2021, `heiwa_evidence` operator JSONL, `heiwa_session` sole-writer service, `heiwa_work` projections, `heiwa_workspace` leases and worktrees, `heiwa_shell` CLI, `std::process`, existing SHA-256 and UUID dependencies. No PTY crate, no new storage engine, no new external dependency.

---

## Scope and release boundary

This closes ledger rows 6 and 7 of Release A1-c. It does **not** close A1.

| Slice | Delivers | Status |
|---|---|---|
| A1-c1 | durable Work-bound turns, runtime-event propagation, Work-session projection, `heiwa work show` | done at `52dca559` |
| A1-c2 (this plan) | provider-owned worker in the prepared worktree; durable pane bound to Work and worker; `runs` collection | this plan |
| A1-c3 | Home/Work/Agent agreement, restart recovery, `scripts/check_work_fabric_a1_acceptance.sh` | deferred |

**Independently useful because:** after this slice a user can start real provider
work inside an isolated worktree and later ask what ran, as what identity, under
which lease, against which base commit, and how it ended — from the journal
alone, with no live process required.

## Design decisions

1. **Identity is appended before the process exists.** `worker_launched` is
   written first. A spawn that then fails appends `worker_exited` with a failure
   code. The reverse order would produce a running process no replay knows about.
2. **A pane is not a worker.** The spec's non-goals forbid treating a terminal
   pane as a verified worker because it exists. `pane_opened` carries the
   `worker_id` it is bound to, and a pane whose worker never reached `Live` folds
   as `unverified`, never as `working`.
3. **Executable identity is recorded, not trusted.** The launch records the
   resolved absolute path and the SHA-256 of the file it actually opened. A
   provider name alone is not identity.
4. **The lease names the worker.** `PersistedWorkerLease.session_id` currently
   repeats `work_id` with a comment saying A1-c will carry a worker session. It
   now carries `worker_id`. `task_id` keeps carrying `work_id`.
5. **Output is bounded and redacted at the boundary.** The pane keeps the last
   `PANE_TAIL_LINES` lines, each truncated to `PANE_LINE_BYTES`. Full logs are
   never operator-journal content — the existing `find_sensitive` screen still
   runs on every append.
6. **No interactive pane operations.** `send`, `split`, `focus`, `pause`, and
   `resume` from the spec's Terminal Runtime contract need a PTY adapter and are
   explicitly out of scope; `create`, `read`, and `stop` are in.
7. **No restart reattach.** Row 9 is A1-c3. A worker whose process is gone folds
   as `stale`, which is honest, rather than being reattached.

## Existing substrate

- `crates/heiwa_evidence/src/operator.rs` — `OperatorEventType`, optional
  `work_id` on every event, cursor-ordered paged replay.
- `crates/heiwa_evidence/src/records.rs:93` — `PersistedWorkerLease` with the
  `session_id` field this plan fills.
- `crates/heiwa_session/src/operator.rs:615` — `append_event`, the sole writer.
- `crates/heiwa_workspace/src/lease.rs:46` — `acquire_writer_lease`, whose
  in-code comment names this slice as the one that supplies a worker session.
- `crates/heiwa_workspace/src/worktree.rs:23` — `WorktreeHandle { work_id, path,
  branch, base_commit }`.
- `crates/heiwa_work/src/session.rs` — `build_work_session`, `COLLECTIONS`, and
  `bounded_upsert`.
- `apps/heiwa_shell/src/cmd/workspace.rs:130` — `prepare_for`, which already
  resolves the runtime root, takes the lease, and appends `workspace_prepared`.

---

## Task 1: Worker and pane event types

**Files:**
- Modify: `crates/heiwa_evidence/src/operator.rs`
- Test: `crates/heiwa_evidence/tests/operator_journal.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/heiwa_evidence/tests/operator_journal.rs`:

```rust
#[test]
fn worker_and_pane_event_types_round_trip_through_json() {
    for (variant, wire) in [
        (OperatorEventType::WorkerLaunched, "worker_launched"),
        (OperatorEventType::WorkerHeartbeat, "worker_heartbeat"),
        (OperatorEventType::WorkerExited, "worker_exited"),
        (OperatorEventType::PaneOpened, "pane_opened"),
        (OperatorEventType::PaneClosed, "pane_closed"),
    ] {
        let encoded = serde_json::to_string(&variant).expect("encode");
        assert_eq!(encoded, format!("\"{wire}\""));
        let decoded: OperatorEventType = serde_json::from_str(&encoded).expect("decode");
        assert_eq!(decoded, variant);
    }
}
```

- [ ] **Step 2: Run the test and confirm it fails**

```bash
cargo test -p heiwa_evidence --test operator worker_and_pane_event_types -- --nocapture
```

Expected: compile error, `no variant named WorkerLaunched`.

- [ ] **Step 3: Add the variants**

In `crates/heiwa_evidence/src/operator.rs`, extend `OperatorEventType` after
`WorkspaceReleased`:

```rust
    WorkspacePrepared,
    WorkspaceReleased,
    WorkerLaunched,
    WorkerHeartbeat,
    WorkerExited,
    PaneOpened,
    PaneClosed,
}
```

The enum already carries `#[serde(rename_all = "snake_case")]`; confirm that
attribute is present before relying on the wire names above, and add it only if
the existing variants already serialize as snake_case strings.

- [ ] **Step 4: Run the test and confirm it passes**

```bash
cargo test -p heiwa_evidence --test operator worker_and_pane_event_types -- --nocapture
```

Expected: `test result: ok. 1 passed`.

- [ ] **Step 5: Confirm nothing else broke**

```bash
cargo test -p heiwa_evidence --locked
```

Expected: all tests pass. A non-exhaustive `match` on `OperatorEventType`
anywhere will fail here; fix each by adding the new arms with the same
behaviour the other domain events get.

- [ ] **Step 6: Commit**

```bash
git add crates/heiwa_evidence/src/operator.rs crates/heiwa_evidence/tests/operator_journal.rs
git commit -m "feat(evidence): add worker and pane operator event types"
```

---

## Task 2: The `heiwa_worker` crate — identity and state

**Files:**
- Create: `crates/heiwa_worker/Cargo.toml`
- Create: `crates/heiwa_worker/src/lib.rs`
- Create: `crates/heiwa_worker/src/model.rs`
- Modify: `Cargo.toml`
- Test: `crates/heiwa_worker/src/model.rs` (inline `mod tests`)

- [ ] **Step 1: Register the crate**

In the root `Cargo.toml`, add to `workspace.members`, keeping alphabetical order
among the `crates/` entries:

```toml
    "crates/heiwa_work",
    "crates/heiwa_worker",
    "crates/heiwa_workspace",
```

- [ ] **Step 2: Write the manifest**

Create `crates/heiwa_worker/Cargo.toml`:

There is no `[workspace.dependencies]` table in this repository; crates pin
their own versions. These are the pins `crates/heiwa_work/Cargo.toml` uses.

```toml
[package]
name = "heiwa_worker"
version = "0.1.0"
edition = "2021"
license.workspace = true
repository.workspace = true
homepage.workspace = true
documentation.workspace = true
readme.workspace = true
keywords.workspace = true
categories.workspace = true
description = "Provider-owned worker and durable terminal pane identity for Heiwa Work."

[dependencies]
heiwa_evidence = { path = "../heiwa_evidence" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
thiserror = "2"

[dev-dependencies]
tempfile = "3"
```

- [ ] **Step 3: Write the failing test**

Create `crates/heiwa_worker/src/model.rs`:

```rust
//! Worker and pane identity. I/O-free: this crate never spawns, reads, or
//! writes. The shell owns the process; this owns what the process *is*.

use serde::{Deserialize, Serialize};

pub const SCHEMA_VERSION: u32 = 1;

/// How a worker looks to the operator right now.
///
/// `Live` is a claim the runtime can defend: identity was appended, the process
/// started, and its last heartbeat is within tolerance. Everything weaker is
/// named as weaker, because the spec forbids treating a process as a verified
/// worker merely because it exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerState {
    Starting,
    Live,
    Stale,
    Exited,
    Failed,
    Revoked,
}

/// What a pane is showing. A pane bound to a worker that never reached `Live`
/// is `Unverified`, never `Working`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaneState {
    Unverified,
    Working,
    Done,
    Failed,
    Stale,
}

impl PaneState {
    pub fn for_worker(worker: WorkerState) -> Self {
        match worker {
            WorkerState::Starting => PaneState::Unverified,
            WorkerState::Live => PaneState::Working,
            WorkerState::Exited => PaneState::Done,
            WorkerState::Failed | WorkerState::Revoked => PaneState::Failed,
            WorkerState::Stale => PaneState::Stale,
        }
    }
}

/// Everything a verified worker records, per the spec's "Legitimate Workers".
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerIdentity {
    pub schema_version: u32,
    pub worker_id: String,
    pub work_id: String,
    pub thread_id: String,
    /// Provider name only. Never a token, cookie, or session secret.
    pub provider: String,
    /// Non-secret reference to a provider-owned session, when one exists.
    pub provider_session_ref: Option<String>,
    /// Absolute path of the executable that was actually opened.
    pub executable_path: String,
    /// SHA-256 of that file's bytes at launch time.
    pub executable_sha256: String,
    /// The prepared worktree. This is the execution location, not a hint.
    pub cwd: String,
    pub repo_root: String,
    pub branch: String,
    pub base_commit: String,
    /// The A1-b writer lease this worker executes under.
    pub lease_id: String,
    pub installation_id: String,
    pub started_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneIdentity {
    pub schema_version: u32,
    pub pane_id: String,
    pub work_id: String,
    pub worker_id: String,
    pub cwd: String,
    pub repo_root: String,
    pub branch: String,
    pub opened_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_worker_that_never_went_live_gives_an_unverified_pane() {
        assert_eq!(
            PaneState::for_worker(WorkerState::Starting),
            PaneState::Unverified
        );
        assert_eq!(PaneState::for_worker(WorkerState::Live), PaneState::Working);
        assert_eq!(PaneState::for_worker(WorkerState::Exited), PaneState::Done);
        assert_eq!(
            PaneState::for_worker(WorkerState::Revoked),
            PaneState::Failed
        );
        assert_eq!(PaneState::for_worker(WorkerState::Stale), PaneState::Stale);
    }

    #[test]
    fn worker_identity_round_trips() {
        let identity = WorkerIdentity {
            schema_version: SCHEMA_VERSION,
            worker_id: "worker-1".into(),
            work_id: "work-1".into(),
            thread_id: "thread-1".into(),
            provider: "claude".into(),
            provider_session_ref: None,
            executable_path: "/usr/local/bin/claude".into(),
            executable_sha256: "a".repeat(64),
            cwd: "/tmp/worktrees/work-1".into(),
            repo_root: "/tmp/repo".into(),
            branch: "heiwa/work-1".into(),
            base_commit: "b".repeat(40),
            lease_id: "lease-1".into(),
            installation_id: "install-1".into(),
            started_at: "2026-08-26T00:00:00Z".into(),
        };
        let encoded = serde_json::to_value(&identity).expect("encode");
        let decoded: WorkerIdentity = serde_json::from_value(encoded).expect("decode");
        assert_eq!(decoded, identity);
    }
}
```

Create `crates/heiwa_worker/src/lib.rs`:

```rust
//! Provider-owned worker and durable terminal pane identity, bound to `Work`.
//!
//! I/O-free by construction, like `heiwa_work`: the shell spawns processes and
//! appends events, this crate says what a worker and a pane *are* and folds the
//! operator stream into the `runs` read model. See
//! `docs/superpowers/specs/2026-08-22-heiwa-work-fabric-design.md`.

pub mod model;

pub use model::{PaneIdentity, PaneState, WorkerIdentity, WorkerState, SCHEMA_VERSION};

#[derive(Debug, thiserror::Error)]
pub enum WorkerError {
    #[error("worker record is schema version {0}, newer than this build understands")]
    UnknownVersion(u32),
    #[error("{0}")]
    Malformed(String),
}

pub type Result<T> = std::result::Result<T, WorkerError>;
```

- [ ] **Step 4: Run the tests and confirm they pass**

```bash
cargo test -p heiwa_worker --locked
```

Expected: `test result: ok. 2 passed`.

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/heiwa_worker/
git commit -m "feat(worker): add worker and pane identity crate"
```

---

## Task 3: Worker and pane event builders

**Files:**
- Create: `crates/heiwa_worker/src/events.rs`
- Modify: `crates/heiwa_worker/src/lib.rs`
- Test: `crates/heiwa_worker/src/events.rs` (inline `mod tests`)

- [ ] **Step 1: Write the failing test**

Create `crates/heiwa_worker/src/events.rs` with the builders and this test at
the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn identity() -> WorkerIdentity {
        WorkerIdentity {
            schema_version: SCHEMA_VERSION,
            worker_id: "worker-1".into(),
            work_id: "work-1".into(),
            thread_id: "thread-1".into(),
            provider: "claude".into(),
            provider_session_ref: None,
            executable_path: "/usr/local/bin/claude".into(),
            executable_sha256: "a".repeat(64),
            cwd: "/tmp/worktrees/work-1".into(),
            repo_root: "/tmp/repo".into(),
            branch: "heiwa/work-1".into(),
            base_commit: "b".repeat(40),
            lease_id: "lease-1".into(),
            installation_id: "install-1".into(),
            started_at: "2026-08-26T00:00:00Z".into(),
        }
    }

    #[test]
    fn launch_event_carries_work_scope_on_the_envelope() {
        let event = worker_launched_event(&identity(), "2026-08-26T00:00:00Z", || "e1".into());
        assert_eq!(event.work_id.as_deref(), Some("work-1"));
        assert_eq!(event.thread_id, "thread-1");
        assert_eq!(event.event_type, OperatorEventType::WorkerLaunched);
        // Ownership must be readable from the envelope alone.
        assert_eq!(
            WorkerLaunchedPayload::from_event(&event).expect("payload").worker_id,
            "worker-1"
        );
    }

    #[test]
    fn the_actor_is_the_worker_not_the_human() {
        let event = worker_launched_event(&identity(), "2026-08-26T00:00:00Z", || "e1".into());
        assert_eq!(event.actor.kind, "worker");
        assert_eq!(event.actor.id, "worker-1");
    }

    #[test]
    fn payload_readers_refuse_the_wrong_event_type() {
        let event = worker_launched_event(&identity(), "2026-08-26T00:00:00Z", || "e1".into());
        assert!(WorkerExitedPayload::from_event(&event).is_none());
    }
}
```

- [ ] **Step 2: Run it and confirm it fails**

```bash
cargo test -p heiwa_worker events -- --nocapture
```

Expected: compile error, `cannot find function worker_launched_event`.

- [ ] **Step 3: Write the builders**

Above that test module in `crates/heiwa_worker/src/events.rs`:

```rust
//! Worker and pane facts as operator-domain events.
//!
//! These join the one operator stream rather than a second store, so replaying
//! one Work produces the whole of it — including who ran, where, and how it
//! ended. The actor is the worker, never the human: the spec forbids a worker
//! claiming the human actor.

use heiwa_evidence::{
    OperatorActor, OperatorEvent, OperatorEventType, OperatorRisk, OperatorSensitivity,
    OPERATOR_EVENT_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};

use crate::model::{PaneIdentity, WorkerIdentity, SCHEMA_VERSION};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerLaunchedPayload {
    pub worker_id: String,
    pub provider: String,
    pub provider_session_ref: Option<String>,
    pub executable_path: String,
    pub executable_sha256: String,
    pub cwd: String,
    pub repo_root: String,
    pub branch: String,
    pub base_commit: String,
    pub lease_id: String,
    pub installation_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerHeartbeatPayload {
    pub worker_id: String,
    pub pid: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerExitedPayload {
    pub worker_id: String,
    /// `None` when the process was signalled or never started.
    pub exit_code: Option<i32>,
    /// Set when the worker failed rather than completing.
    pub failure_code: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneOpenedPayload {
    pub pane_id: String,
    pub worker_id: String,
    pub cwd: String,
    pub repo_root: String,
    pub branch: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneClosedPayload {
    pub pane_id: String,
    pub worker_id: String,
    /// Bounded, truncated tail of what the pane showed. Never the full log.
    pub tail: Vec<String>,
    /// Lines dropped before `tail` begins, so the reader knows it is a tail.
    pub dropped_lines: usize,
}

macro_rules! from_event {
    ($ty:ty, $variant:expr) => {
        impl $ty {
            pub fn from_event(event: &OperatorEvent) -> Option<Self> {
                if event.event_type != $variant {
                    return None;
                }
                serde_json::from_value(event.payload.clone()).ok()
            }
        }
    };
}

from_event!(WorkerLaunchedPayload, OperatorEventType::WorkerLaunched);
from_event!(WorkerHeartbeatPayload, OperatorEventType::WorkerHeartbeat);
from_event!(WorkerExitedPayload, OperatorEventType::WorkerExited);
from_event!(PaneOpenedPayload, OperatorEventType::PaneOpened);
from_event!(PaneClosedPayload, OperatorEventType::PaneClosed);

/// One worker-authored event, scoped to its Work and thread.
fn worker_scoped(
    work_id: &str,
    thread_id: &str,
    worker_id: &str,
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
        run_id: Some(worker_id.to_string()),
        call_id: None,
        work_id: Some(work_id.to_string()),
        event_type,
        occurred_at: occurred_at.to_string(),
        actor: OperatorActor {
            kind: "worker".to_string(),
            id: worker_id.to_string(),
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

pub fn worker_launched_event(
    identity: &WorkerIdentity,
    occurred_at: &str,
    new_event_id: impl FnOnce() -> String,
) -> OperatorEvent {
    debug_assert_eq!(identity.schema_version, SCHEMA_VERSION);
    let payload = serde_json::to_value(WorkerLaunchedPayload {
        worker_id: identity.worker_id.clone(),
        provider: identity.provider.clone(),
        provider_session_ref: identity.provider_session_ref.clone(),
        executable_path: identity.executable_path.clone(),
        executable_sha256: identity.executable_sha256.clone(),
        cwd: identity.cwd.clone(),
        repo_root: identity.repo_root.clone(),
        branch: identity.branch.clone(),
        base_commit: identity.base_commit.clone(),
        lease_id: identity.lease_id.clone(),
        installation_id: identity.installation_id.clone(),
    })
    .expect("worker launch payload is plain data");
    worker_scoped(
        &identity.work_id,
        &identity.thread_id,
        &identity.worker_id,
        OperatorEventType::WorkerLaunched,
        occurred_at,
        payload,
        new_event_id,
    )
}

pub fn worker_heartbeat_event(
    identity: &WorkerIdentity,
    pid: u32,
    occurred_at: &str,
    new_event_id: impl FnOnce() -> String,
) -> OperatorEvent {
    let payload = serde_json::to_value(WorkerHeartbeatPayload {
        worker_id: identity.worker_id.clone(),
        pid,
    })
    .expect("worker heartbeat payload is plain data");
    worker_scoped(
        &identity.work_id,
        &identity.thread_id,
        &identity.worker_id,
        OperatorEventType::WorkerHeartbeat,
        occurred_at,
        payload,
        new_event_id,
    )
}

pub fn worker_exited_event(
    identity: &WorkerIdentity,
    exit_code: Option<i32>,
    failure_code: Option<String>,
    occurred_at: &str,
    new_event_id: impl FnOnce() -> String,
) -> OperatorEvent {
    let payload = serde_json::to_value(WorkerExitedPayload {
        worker_id: identity.worker_id.clone(),
        exit_code,
        failure_code,
    })
    .expect("worker exit payload is plain data");
    worker_scoped(
        &identity.work_id,
        &identity.thread_id,
        &identity.worker_id,
        OperatorEventType::WorkerExited,
        occurred_at,
        payload,
        new_event_id,
    )
}

pub fn pane_opened_event(
    pane: &PaneIdentity,
    thread_id: &str,
    occurred_at: &str,
    new_event_id: impl FnOnce() -> String,
) -> OperatorEvent {
    let payload = serde_json::to_value(PaneOpenedPayload {
        pane_id: pane.pane_id.clone(),
        worker_id: pane.worker_id.clone(),
        cwd: pane.cwd.clone(),
        repo_root: pane.repo_root.clone(),
        branch: pane.branch.clone(),
    })
    .expect("pane open payload is plain data");
    worker_scoped(
        &pane.work_id,
        thread_id,
        &pane.worker_id,
        OperatorEventType::PaneOpened,
        occurred_at,
        payload,
        new_event_id,
    )
}

pub fn pane_closed_event(
    pane: &PaneIdentity,
    thread_id: &str,
    tail: Vec<String>,
    dropped_lines: usize,
    occurred_at: &str,
    new_event_id: impl FnOnce() -> String,
) -> OperatorEvent {
    let payload = serde_json::to_value(PaneClosedPayload {
        pane_id: pane.pane_id.clone(),
        worker_id: pane.worker_id.clone(),
        tail,
        dropped_lines,
    })
    .expect("pane close payload is plain data");
    worker_scoped(
        &pane.work_id,
        thread_id,
        &pane.worker_id,
        OperatorEventType::PaneClosed,
        occurred_at,
        payload,
        new_event_id,
    )
}
```

Add to `crates/heiwa_worker/src/lib.rs`:

```rust
pub mod events;

pub use events::{
    pane_closed_event, pane_opened_event, worker_exited_event, worker_heartbeat_event,
    worker_launched_event, PaneClosedPayload, PaneOpenedPayload, WorkerExitedPayload,
    WorkerHeartbeatPayload, WorkerLaunchedPayload,
};
```

- [ ] **Step 4: Run and confirm the tests pass**

```bash
cargo test -p heiwa_worker --locked
```

Expected: `test result: ok. 5 passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/heiwa_worker/
git commit -m "feat(worker): add worker and pane event builders"
```

---

## Task 4: Bounded pane tail

**Files:**
- Create: `crates/heiwa_worker/src/pane.rs`
- Modify: `crates/heiwa_worker/src/lib.rs`
- Test: `crates/heiwa_worker/src/pane.rs` (inline `mod tests`)

- [ ] **Step 1: Write the failing test**

Create `crates/heiwa_worker/src/pane.rs` with this test at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tail_keeps_only_the_last_lines_and_counts_what_it_dropped() {
        let mut tail = PaneTail::new(3, 100);
        for line in ["one", "two", "three", "four", "five"] {
            tail.push(line);
        }
        assert_eq!(tail.lines(), ["three", "four", "five"]);
        assert_eq!(tail.dropped_lines(), 2);
    }

    #[test]
    fn a_long_line_is_truncated_on_a_character_boundary() {
        let mut tail = PaneTail::new(4, 8);
        tail.push("aaaaaaaaaaaaaaaa");
        assert_eq!(tail.lines(), ["aaaaaaaa"]);
    }

    #[test]
    fn truncation_never_splits_a_multibyte_character() {
        let mut tail = PaneTail::new(4, 5);
        // Four 3-byte characters: a naive byte slice at 5 would panic.
        tail.push("日本語だ");
        let kept = &tail.lines()[0];
        assert!(kept.len() <= 5, "kept {} bytes", kept.len());
        assert_eq!(kept, "日");
    }

    #[test]
    fn an_empty_tail_reports_nothing_dropped() {
        let tail = PaneTail::new(3, 100);
        assert!(tail.lines().is_empty());
        assert_eq!(tail.dropped_lines(), 0);
    }
}
```

- [ ] **Step 2: Run it and confirm it fails**

```bash
cargo test -p heiwa_worker pane -- --nocapture
```

Expected: compile error, `cannot find type PaneTail`.

- [ ] **Step 3: Implement the tail**

Above that test module:

```rust
//! The bounded window a pane keeps of its worker's output.
//!
//! Full terminal logs are never operator-journal content: the spec routes them
//! to disposable signal frames. What lands durably is a tail plus an honest
//! count of what was dropped, so a reader can never mistake it for the whole
//! log.

use std::collections::VecDeque;

/// Lines a pane keeps by default.
pub const PANE_TAIL_LINES: usize = 200;
/// Bytes each kept line is truncated to.
pub const PANE_LINE_BYTES: usize = 2_000;

#[derive(Clone, Debug)]
pub struct PaneTail {
    lines: VecDeque<String>,
    capacity: usize,
    line_bytes: usize,
    dropped: usize,
}

impl PaneTail {
    pub fn new(capacity: usize, line_bytes: usize) -> Self {
        Self {
            lines: VecDeque::new(),
            capacity: capacity.max(1),
            line_bytes: line_bytes.max(1),
            dropped: 0,
        }
    }

    pub fn push(&mut self, line: &str) {
        self.lines.push_back(truncate_on_char_boundary(line, self.line_bytes));
        while self.lines.len() > self.capacity {
            self.lines.pop_front();
            self.dropped += 1;
        }
    }

    pub fn lines(&self) -> Vec<String> {
        self.lines.iter().cloned().collect()
    }

    pub fn dropped_lines(&self) -> usize {
        self.dropped
    }
}

impl Default for PaneTail {
    fn default() -> Self {
        Self::new(PANE_TAIL_LINES, PANE_LINE_BYTES)
    }
}

/// Truncate to at most `max_bytes`, never splitting a UTF-8 character.
fn truncate_on_char_boundary(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}
```

Add to `crates/heiwa_worker/src/lib.rs`:

```rust
pub mod pane;

pub use pane::{PaneTail, PANE_LINE_BYTES, PANE_TAIL_LINES};
```

- [ ] **Step 4: Run and confirm the tests pass**

```bash
cargo test -p heiwa_worker --locked
```

Expected: `test result: ok. 9 passed`.

- [ ] **Step 5: Commit**

```bash
git add crates/heiwa_worker/
git commit -m "feat(worker): add bounded, char-safe pane tail"
```

---

## Task 5: The `runs` fold

**Files:**
- Create: `crates/heiwa_worker/src/projector.rs`
- Modify: `crates/heiwa_worker/src/lib.rs`
- Test: `crates/heiwa_worker/tests/runs.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/heiwa_worker/tests/runs.rs`:

```rust
use heiwa_evidence::{OperatorEvent, OperatorEventType};
use heiwa_worker::{
    fold_runs, pane_closed_event, pane_opened_event, worker_exited_event, worker_launched_event,
    PaneIdentity, PaneState, WorkerIdentity, WorkerState, SCHEMA_VERSION,
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
fn a_launched_worker_that_has_not_exited_is_starting() {
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
fn a_clean_exit_is_exited_and_a_failure_code_is_failed() {
    let mut next = ids();
    let id = identity("work-1", "worker-1");
    let clean = vec![
        worker_launched_event(&id, "2026-08-26T00:00:00Z", &mut next),
        worker_exited_event(&id, Some(0), None, "2026-08-26T00:00:01Z", &mut next),
    ];
    assert_eq!(fold_runs(&clean, "work-1")[0].worker_state, WorkerState::Exited);

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
    assert_eq!(fold_runs(&broken, "work-1")[0].worker_state, WorkerState::Failed);
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
        worker_launched_event(&identity("work-1", "worker-1"), "2026-08-26T00:00:00Z", &mut next),
        worker_launched_event(&identity("work-2", "worker-2"), "2026-08-26T00:00:01Z", &mut next),
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
    let mut event = worker_launched_event(&identity("work-1", "worker-1"), "2026-08-26T00:00:00Z", &mut next);
    event.payload = serde_json::json!({ "worker_id": "worker-1", "provider": "claude" });
    // A payload that no longer deserializes fully must not silently vanish; the
    // envelope still says this Work has a worker.
    let runs = fold_runs(&[event], "work-1");
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].worker_id, "worker-1");
}
```

- [ ] **Step 2: Run it and confirm it fails**

```bash
cargo test -p heiwa_worker --test runs -- --nocapture
```

Expected: compile error, `cannot find function fold_runs`.

- [ ] **Step 3: Implement the fold**

Create `crates/heiwa_worker/src/projector.rs`:

```rust
//! Pure fold from the operator stream to the `runs` read model.
//!
//! Append order is authority. A row is keyed by `worker_id` and only ever
//! created by a `worker_launched` envelope scoped to the requested Work — a
//! pane alone never mints one.

use std::collections::BTreeMap;

use heiwa_evidence::{OperatorEvent, OperatorEventType};
use serde::{Deserialize, Serialize};

use crate::events::{
    PaneClosedPayload, PaneOpenedPayload, WorkerExitedPayload, WorkerLaunchedPayload,
};
use crate::model::{PaneState, WorkerState};

/// One worker run inside one Work, with the pane bound to it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRow {
    pub worker_id: String,
    pub work_id: String,
    pub worker_state: WorkerState,
    pub provider: Option<String>,
    pub provider_session_ref: Option<String>,
    pub executable_path: Option<String>,
    pub executable_sha256: Option<String>,
    pub cwd: Option<String>,
    pub repo_root: Option<String>,
    pub branch: Option<String>,
    pub base_commit: Option<String>,
    pub lease_id: Option<String>,
    pub started_at: Option<String>,
    pub ended_at: Option<String>,
    pub exit_code: Option<i32>,
    pub failure_code: Option<String>,
    pub pane_id: Option<String>,
    pub pane_state: Option<PaneState>,
    pub pane_tail: Vec<String>,
    pub pane_dropped_lines: usize,
}

impl RunRow {
    fn new(worker_id: String, work_id: String) -> Self {
        Self {
            worker_id,
            work_id,
            worker_state: WorkerState::Starting,
            provider: None,
            provider_session_ref: None,
            executable_path: None,
            executable_sha256: None,
            cwd: None,
            repo_root: None,
            branch: None,
            base_commit: None,
            lease_id: None,
            started_at: None,
            ended_at: None,
            exit_code: None,
            failure_code: None,
            pane_id: None,
            pane_state: None,
            pane_tail: Vec::new(),
            pane_dropped_lines: 0,
        }
    }
}

/// Fold `events` into the run rows belonging to `work_id`, in first-launch
/// order.
pub fn fold_runs(events: &[OperatorEvent], work_id: &str) -> Vec<RunRow> {
    let mut rows: BTreeMap<String, RunRow> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();

    for event in events {
        if event.work_id.as_deref() != Some(work_id) {
            continue;
        }
        match event.event_type {
            OperatorEventType::WorkerLaunched => {
                // The envelope's run_id is the worker; the payload only enriches.
                let worker_id = match event.run_id.clone() {
                    Some(id) => id,
                    None => continue,
                };
                if !rows.contains_key(&worker_id) {
                    order.push(worker_id.clone());
                    rows.insert(
                        worker_id.clone(),
                        RunRow::new(worker_id.clone(), work_id.to_string()),
                    );
                }
                let row = rows.get_mut(&worker_id).expect("just inserted");
                row.started_at = Some(event.occurred_at.clone());
                if let Some(payload) = WorkerLaunchedPayload::from_event(event) {
                    row.provider = Some(payload.provider);
                    row.provider_session_ref = payload.provider_session_ref;
                    row.executable_path = Some(payload.executable_path);
                    row.executable_sha256 = Some(payload.executable_sha256);
                    row.cwd = Some(payload.cwd);
                    row.repo_root = Some(payload.repo_root);
                    row.branch = Some(payload.branch);
                    row.base_commit = Some(payload.base_commit);
                    row.lease_id = Some(payload.lease_id);
                }
            }
            OperatorEventType::WorkerHeartbeat => {
                if let Some(id) = event.run_id.as_deref() {
                    if let Some(row) = rows.get_mut(id) {
                        if row.worker_state == WorkerState::Starting {
                            row.worker_state = WorkerState::Live;
                        }
                    }
                }
            }
            OperatorEventType::WorkerExited => {
                let Some(id) = event.run_id.as_deref() else { continue };
                let Some(row) = rows.get_mut(id) else { continue };
                row.ended_at = Some(event.occurred_at.clone());
                if let Some(payload) = WorkerExitedPayload::from_event(event) {
                    row.exit_code = payload.exit_code;
                    row.failure_code = payload.failure_code.clone();
                    row.worker_state = if payload.failure_code.is_some() {
                        WorkerState::Failed
                    } else {
                        WorkerState::Exited
                    };
                } else {
                    row.worker_state = WorkerState::Exited;
                }
                if row.pane_id.is_some() {
                    row.pane_state = Some(PaneState::for_worker(row.worker_state));
                }
            }
            OperatorEventType::PaneOpened => {
                let Some(payload) = PaneOpenedPayload::from_event(event) else { continue };
                // A pane never mints a run: an unlaunched worker stays unknown.
                let Some(row) = rows.get_mut(&payload.worker_id) else { continue };
                row.pane_id = Some(payload.pane_id);
                row.pane_state = Some(PaneState::for_worker(row.worker_state));
            }
            OperatorEventType::PaneClosed => {
                let Some(payload) = PaneClosedPayload::from_event(event) else { continue };
                let Some(row) = rows.get_mut(&payload.worker_id) else { continue };
                row.pane_tail = payload.tail;
                row.pane_dropped_lines = payload.dropped_lines;
                row.pane_state = Some(PaneState::for_worker(row.worker_state));
            }
            _ => {}
        }
    }

    order
        .into_iter()
        .filter_map(|id| rows.remove(&id))
        .collect()
}
```

Add to `crates/heiwa_worker/src/lib.rs`:

```rust
pub mod projector;

pub use projector::{fold_runs, RunRow};
```

- [ ] **Step 4: Run and confirm the tests pass**

```bash
cargo test -p heiwa_worker --locked
```

Expected: all tests pass, including the seven in `runs.rs`.

- [ ] **Step 5: Commit**

```bash
git add crates/heiwa_worker/
git commit -m "feat(worker): fold worker and pane events into runs rows"
```

---

## Task 6: The writer lease names the worker

**Files:**
- Modify: `crates/heiwa_workspace/src/lease.rs`
- Modify: `apps/heiwa_shell/src/cmd/workspace.rs`
- Test: `crates/heiwa_workspace/tests/workspace_core.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/heiwa_workspace/tests/workspace_core.rs`:

```rust
#[test]
fn a_lease_records_the_worker_session_it_was_issued_for() {
    let temp = tempfile::tempdir().expect("tempdir");
    let evidence = temp.path().join("evidence");
    std::fs::create_dir_all(&evidence).expect("evidence dir");
    let transport = heiwa_evidence::JsonlTransport::new(evidence.clone()).expect("transport");

    let lease = heiwa_workspace::acquire_writer_lease(
        &evidence,
        &transport,
        "work-1",
        "/tmp/repo",
        "install-1",
        "worker-1",
        "2026-08-26T00:00:00Z",
        "2026-08-26T08:00:00Z",
        || "lease-1".to_string(),
    )
    .expect("lease");

    assert_eq!(lease.worker_id, "worker-1");

    let view = heiwa_evidence::WorkerStateView::replay(&evidence).expect("replay");
    let persisted = view.leases.get("lease-1").expect("persisted lease");
    // task_id keeps naming the Work; session_id now names the worker rather
    // than repeating the Work, which is the seam A1-b left for A1-c.
    assert_eq!(persisted.task_id, "work-1");
    assert_eq!(persisted.session_id, "worker-1");
}
```

- [ ] **Step 2: Run it and confirm it fails**

```bash
cargo test -p heiwa_workspace --test workspace_core a_lease_records -- --nocapture
```

Expected: compile error — `acquire_writer_lease` takes fewer arguments, and
`WriterLease` has no `worker_id`.

- [ ] **Step 3: Thread the worker through**

In `crates/heiwa_workspace/src/lease.rs`, add the field to `WriterLease`:

```rust
pub struct WriterLease {
    pub lease_id: String,
    pub work_id: String,
    /// The worker session this lease was issued for.
    pub worker_id: String,
    pub capability: String,
    pub node_id: String,
    pub issued_at: String,
    pub expires_at: String,
}
```

Add a `worker_id: &str` parameter to `acquire_writer_lease` immediately after
`installation_id`, set it on the returned `WriterLease`, and replace the
placeholder in `PersistedWorkerLease`:

```rust
        session_id: worker_id.to_string(),
```

Delete the two-line comment above it that says A1-b has no separate worker
session, since it now does.

`release_writer_lease` and `revoke_writer_lease` rebuild the persisted record;
make sure both carry `session_id` forward from the lease they are given rather
than re-deriving it, since replay keeps only the last record per `lease_id`.

- [ ] **Step 4: Update the one caller**

In `apps/heiwa_shell/src/cmd/workspace.rs`, `prepare_for` currently has no
worker. Give it one, so the lease is attributable from the moment it exists:
add a `worker_id: &str` parameter to `prepare_for`, pass it to
`acquire_writer_lease`, and at the `prepare` call site generate it with
`uuid::Uuid::new_v4()` prefixed `worker-`. Task 7 replaces that generation with
one shared identity, so keep it in a single `let worker_id = ...;` binding.

- [ ] **Step 5: Run and confirm the tests pass**

```bash
cargo test -p heiwa_workspace --locked
cargo test -p heiwa-shell --bin heiwa cmd::workspace
```

Expected: both green.

- [ ] **Step 6: Commit**

```bash
git add crates/heiwa_workspace/ apps/heiwa_shell/src/cmd/workspace.rs
git commit -m "feat(workspace): bind the writer lease to a worker session"
```

---

## Task 7: `heiwa work run` launches the worker in the worktree

**Files:**
- Create: `apps/heiwa_shell/src/cmd/worker.rs`
- Modify: `apps/heiwa_shell/src/cmd/mod.rs`
- Modify: `apps/heiwa_shell/src/main.rs`
- Modify: `apps/heiwa_shell/Cargo.toml`
- Test: `apps/heiwa_shell/src/cmd/worker.rs` (inline `mod tests`)

- [ ] **Step 1: Write the failing test**

At the bottom of `apps/heiwa_shell/src/cmd/worker.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_executable_is_identified_by_path_and_content_digest() {
        let temp = tempfile::tempdir().expect("tempdir");
        let script = temp.path().join("fake-provider");
        std::fs::write(&script, b"#!/bin/sh\nexit 0\n").expect("write");

        let identified = identify_executable(&script).expect("identify");
        assert_eq!(identified.path, script.canonicalize().expect("canon").display().to_string());
        // sha256 of the exact bytes written above.
        assert_eq!(identified.sha256.len(), 64);
        assert!(identified.sha256.chars().all(|c| c.is_ascii_hexdigit()));

        std::fs::write(&script, b"#!/bin/sh\nexit 1\n").expect("rewrite");
        let changed = identify_executable(&script).expect("identify again");
        assert_ne!(changed.sha256, identified.sha256, "digest must follow content");
    }

    #[test]
    fn a_missing_executable_is_refused_before_anything_is_appended() {
        let temp = tempfile::tempdir().expect("tempdir");
        let missing = temp.path().join("not-here");
        assert!(identify_executable(&missing).is_err());
    }

    #[test]
    fn a_worker_id_is_a_safe_path_and_ref_component() {
        let id = new_worker_id(|| "3f2b0c18-0000-4000-8000-000000000000".to_string());
        assert!(id.starts_with("worker-"));
        assert!(!id.contains('/'));
        assert!(!id.contains(".."));
        assert!(id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-'));
    }
}
```

- [ ] **Step 2: Run it and confirm it fails**

```bash
cargo test -p heiwa-shell --bin heiwa cmd::worker -- --nocapture
```

Expected: compile error, `cannot find function identify_executable`.

- [ ] **Step 3: Implement the command**

Create `apps/heiwa_shell/src/cmd/worker.rs`:

```rust
//! `heiwa work run` — start one provider-owned worker inside the worktree that
//! `heiwa workspace prepare` created for a Work.
//!
//! The shell owns the only process spawn in this slice. Identity is appended
//! before the child exists, so a spawn that fails still leaves a record; the
//! reverse order would produce a running process no replay knows about.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use heiwa_evidence::OperatorJournal;
use heiwa_session::operator::OperatorSessionService;
use heiwa_worker::{
    pane_closed_event, pane_opened_event, worker_exited_event, worker_heartbeat_event,
    worker_launched_event, PaneIdentity, PaneTail, WorkerIdentity, SCHEMA_VERSION,
};

pub struct IdentifiedExecutable {
    pub path: String,
    pub sha256: String,
}

/// Resolve an executable to a canonical path and the digest of its bytes.
///
/// A provider name is not identity: two machines' `claude` are different
/// programs, and the same machine's can change under us between runs.
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

/// Worker IDs land in journal paths and lease capability strings, so they are
/// restricted to characters that cannot escape either.
pub fn new_worker_id(new_uuid: impl FnOnce() -> String) -> String {
    let raw = new_uuid();
    let safe: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    format!("worker-{safe}")
}
```

Then add the `run` entry point below it, following the shape of
`apps/heiwa_shell/src/cmd/workspace.rs::prepare`:

1. Parse `heiwa work run <work-id> [--provider <name>] [--json] -- <command>…`.
   Refuse with a usage error when the command after `--` is empty.
2. Resolve `HeiwaPaths` and the local identity exactly as `prepare` does; refuse
   when there is no identity.
3. Call `crate::cmd::work::find` and refuse when the Work is absent or belongs
   to another installation, reusing `prepare`'s message wording.
4. Locate the prepared worktree with `heiwa_workspace::list_worktrees_in`
   against the repository root, matching `work_id`. Refuse with
   `run heiwa workspace prepare <work-id> first` when there is none — this slice
   never prepares implicitly, because preparation takes a lease.
5. `identify_executable` on the first token of the command, resolved through
   `PATH` when it has no separator.
6. Build `WorkerIdentity` with `schema_version: SCHEMA_VERSION`, the worktree's
   `path`/`branch`/`base_commit`, the lease id read back from the worktree's
   `workspace_prepared` event, and `started_at` from `chrono::Utc::now()`.
7. Append `worker_launched_event` through `OperatorSessionService::append_event`.
   **Return the error without spawning if this append fails.**
8. Build `PaneIdentity` and append `pane_opened_event`.
9. Spawn with `Command::new(...).current_dir(&worktree.path)` and
   `.stdout(Stdio::piped()).stderr(Stdio::piped())`. Pass only
   `PATH`, `HOME`, `LANG`, and `TERM` through with `.env_clear()` then
   `.env(...)` for each — the spec gives workers only task-required environment
   values, and inheriting the operator's environment would hand a worker every
   credential in it.
10. On spawn failure, append `worker_exited_event(.., None,
    Some("spawn_failed"), ..)` and return the error.
11. Append one `worker_heartbeat_event` with the child's `id()` as soon as the
    child exists — this is what moves the fold from `Starting` to `Live`.
12. Read stdout and stderr line by line into a `PaneTail::default()`, echoing
    each line to the operator's own stdout so the pane is live, not just
    recorded.
13. `wait()` the child, append `pane_closed_event` with `tail.lines()` and
    `tail.dropped_lines()`, then `worker_exited_event` with the code.
14. Print the worker id, pane id, cwd, branch, and exit code; with `--json`
    print the same as one object.

Add `sha2` to `apps/heiwa_shell/Cargo.toml` dependencies (use the version the
workspace already pins) and `heiwa_worker = { path = "../../crates/heiwa_worker" }`.

- [ ] **Step 4: Wire the subcommand**

In `apps/heiwa_shell/src/cmd/mod.rs` add `pub mod worker;`. In
`apps/heiwa_shell/src/cmd/work.rs::run`, add a `Some("run") => crate::cmd::worker::run(&args[1..])`
arm and a `heiwa work run` line to `print_help`.

- [ ] **Step 5: Run and confirm the tests pass**

```bash
cargo test -p heiwa-shell --bin heiwa cmd::worker -- --nocapture
```

Expected: `3 passed`.

- [ ] **Step 6: Commit**

```bash
git add apps/heiwa_shell/ Cargo.lock
git commit -m "feat(shell): run a provider-owned worker inside the prepared worktree"
```

---

## Task 8: Integration through the real journal

**Files:**
- Create: `apps/heiwa_shell/tests/worker_in_worktree.rs`

- [ ] **Step 1: Write the failing test**

Create `apps/heiwa_shell/tests/worker_in_worktree.rs`. It must, against a real
temporary git repository and a real evidence root:

```rust
// 1. init a repo with one commit
// 2. `heiwa work` create a Work, then `heiwa workspace prepare` it
// 3. `heiwa work run <work-id> -- /bin/sh -c 'pwd; echo hello'`
// 4. assert the child's `pwd` output equals the prepared worktree path,
//    proving the worker ran inside the worktree and not the repo root
// 5. replay the operator journal and assert, in append order:
//    worker_launched, pane_opened, worker_heartbeat, pane_closed, worker_exited
// 6. assert every one of those five events carries work_id on the envelope
// 7. assert `fold_runs` yields exactly one row, Exited, exit_code 0,
//    cwd == worktree path, and a non-empty executable_sha256
```

Write those as concrete `#[test]` functions using the same harness helpers
`apps/heiwa_shell/tests/` already uses for `cmd::workspace` integration; do not
introduce a second harness.

- [ ] **Step 2: Run it and confirm it fails**

```bash
cargo test -p heiwa-shell --test worker_in_worktree -- --nocapture
```

Expected: failure, since `heiwa work run` has no integration coverage yet.

- [ ] **Step 3: Fix whatever it catches**

Do not weaken the assertions. If ordering is wrong, fix the emission order in
`cmd/worker.rs`; the plan's order is normative.

- [ ] **Step 4: Run and confirm it passes**

```bash
cargo test -p heiwa-shell --test worker_in_worktree -- --nocapture
```

- [ ] **Step 5: Commit**

```bash
git add apps/heiwa_shell/tests/worker_in_worktree.rs
git commit -m "test(shell): prove the worker runs inside the prepared worktree"
```

---

## Task 9: `runs` in the Work-session projection

**Files:**
- Modify: `crates/heiwa_work/src/session.rs`
- Modify: `crates/heiwa_work/Cargo.toml`
- Modify: `apps/heiwa_shell/src/cmd/work.rs`
- Test: `crates/heiwa_work/tests/work_session.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/heiwa_work/tests/work_session.rs`:

```rust
#[test]
fn the_session_snapshot_carries_a_runs_collection() {
    // Build a Work with one launched-and-exited worker, then:
    let snapshot = /* build_work_session over those rows */;
    let runs = snapshot.collections.get("runs").expect("runs collection");
    assert_eq!(runs.rows.len(), 1);
    assert_eq!(runs.rows[0]["worker_id"], "worker-1");
    assert_eq!(runs.rows[0]["worker_state"], "exited");
    // The projection is bounded and redacted: no executable bytes, no full log.
    assert!(runs.rows[0].get("pane_tail").is_some());
    assert!(runs.rows[0].get("environment").is_none());
}

#[test]
fn runs_respect_the_collection_limit_like_every_other_collection() {
    // 5 launched workers with collection_limit 2 -> 2 rows and truncated=true.
}
```

Fill both bodies concretely using the helpers already in that file.

- [ ] **Step 2: Run and confirm it fails**

```bash
cargo test -p heiwa_work --test work_session runs -- --nocapture
```

Expected: `runs collection` panic — `COLLECTIONS` has nine names, not ten.

- [ ] **Step 3: Add the collection**

Add `heiwa_worker = { path = "../heiwa_worker" }` to
`crates/heiwa_work/Cargo.toml`. In `crates/heiwa_work/src/session.rs`, extend
`COLLECTIONS` with `"runs"`, and after the existing collection folds, call
`heiwa_worker::fold_runs` over the same events and `bounded_upsert` each row
keyed by `worker_id`, serializing with `serde_json::to_value`. Keep the same
`options.collection_limit` and truncation bookkeeping the other collections use.

This introduces a `heiwa_work` → `heiwa_worker` dependency. That direction is
correct: `heiwa_worker` knows nothing about Work sessions, and both stay
I/O-free. Do not add the reverse edge.

- [ ] **Step 4: Surface it in the CLI**

`heiwa work show <work-id>` already renders the projector. Add a `runs` block
that prints, per row, `worker_id`, `worker_state`, `provider`, `cwd`, and
`exit_code`, matching the formatting of the neighbouring blocks.

- [ ] **Step 5: Run and confirm the tests pass**

```bash
cargo test -p heiwa_work --locked
cargo test -p heiwa-shell --bin heiwa cmd::work
```

- [ ] **Step 6: Commit**

```bash
git add crates/heiwa_work/ apps/heiwa_shell/src/cmd/work.rs Cargo.lock
git commit -m "feat(work): project worker runs into the Work session snapshot"
```

---

## Task 10: CI grouping and ledger

**Files:**
- Modify: `scripts/ci_rust_test_group.sh`
- Modify: `docs/superpowers/ledgers/2026-08-22-work-fabric-task-ledger.md`

- [ ] **Step 1: Add the crate to its test group**

Run the checker to see what it wants:

```bash
bash scripts/ci_rust_test_group.sh --check
```

Expected: a failure naming `heiwa_worker` as an ungrouped crate. Add it to the
same group `heiwa_work` and `heiwa_workspace` are in.

- [ ] **Step 2: Confirm the grouping check passes**

```bash
bash scripts/ci_rust_test_group.sh --check
```

Expected: pass.

- [ ] **Step 3: Move the ledger rows**

In the A1-c table, change rows 6 and 7 from `pending | A1-c2` to:

```markdown
| 6 | Provider-owned worker runs inside the prepared Work workspace | done | `cargo test -p heiwa-shell --test worker_in_worktree` |
| 7 | Durable terminal pane binds to Work and worker identity | done | `cargo test -p heiwa_worker --test runs` |
```

Add to **Deferred with reason**:

```markdown
- Interactive pane operations — `send`, `split`, `focus`, `pause`, `resume` from
  the Terminal Runtime contract need a PTY adapter. A1-c2 delivers `create`,
  `read`, and `stop`; the rest wait for that adapter rather than being faked
  through a pipe that cannot carry them.
- Restart reattach is row 9, in A1-c3. A worker whose process is gone folds as
  `stale` rather than being resurrected.
```

Update **Next experimental slice** to name A1-c3.

- [ ] **Step 4: Confirm the Stop gate still allows a stop**

```bash
bash scripts/hooks/stop_ledger_gate.sh
```

Expected: no output. Rows 8-10 are still `pending`, so A1 does not yet claim
completion. If this prints a block for Work Fabric A1, a row was moved that
should not have been.

- [ ] **Step 5: Full local verification**

```bash
HEIWA_BRANCH_MODE=experimental bash scripts/check_ci_local.sh
```

Expected: `ALL GREEN`.

- [ ] **Step 6: Commit**

```bash
git add scripts/ci_rust_test_group.sh docs/superpowers/ledgers/2026-08-22-work-fabric-task-ledger.md
git commit -m "chore(work): group the worker crate and record A1-c2 in the ledger"
```

---

## Self-review notes

- **Spec coverage.** "Legitimate Workers" — worker ID, provider and non-secret
  session ref, workspace/repo/worktree, executable identity and execution
  location, lease, start time, heartbeat, state — all land in `WorkerIdentity`
  and `RunRow` (Tasks 2, 5). Parent worker, and the tool/filesystem/network/
  budget/action lease split, are **not** delivered: A1-c2 has one lease and no
  child workers, and inventing the fields now would produce something nothing
  can populate. Record that in the ledger's deferral list if a reviewer asks.
- **Terminal Runtime.** `create`, `read`, `stop`, plus workspace/pane/process
  identity and cwd/repo/branch/lease/evidence refs are covered. `attach`,
  `restore`, `send`, `split`, `focus`, `pause`, `resume` are deferred with
  reason in Task 10.
- **Non-goal held.** "Treating a terminal pane … as a verified worker merely
  because it exists" is enforced by the fold test
  `a_pane_for_an_unknown_worker_is_not_promoted_to_a_run` (Task 5).
- **Authority held.** Worker events use `actor.kind = "worker"`, tested in
  Task 3, so a worker cannot claim the human actor.
- **Type consistency.** `worker_id`, `work_id`, `pane_id`, `lease_id`,
  `executable_sha256`, `fold_runs`, `RunRow`, `PaneTail` are spelled the same in
  every task that touches them.

---

## What actually shipped

Implemented at `fb5f56b3`. Where the tree diverged from the plan above, the
tree is right and this section is the record.

1. **Worker identity is minted at `prepare`, not at `run`.** The plan had Task 6
   generate a throwaway id and Task 7 "replace that generation with one shared
   identity" without saying how. The realization: one prepared workspace serves
   one worker, so `heiwa workspace prepare` mints the id, binds the lease to it,
   and records it on `workspace_prepared`; `heiwa work run` adopts it. This
   required two additive optional payload fields — `worker_id` and `lease_id` —
   on `WorkspacePreparedPayload`, both `#[serde(default)]` so A1-b events
   already on disk keep deserializing. A Work prepared before this change is
   refused with an instruction to re-prepare, rather than run under an invented
   identity no lease knows.
2. **`heiwa_worker` does not depend on `sha2`.** The digest is computed in the
   shell, which is the only place that opens a file. The crate stays I/O-free.
3. **Integration tests live inline in `apps/heiwa_shell/src/cmd/worker.rs`, not
   in `apps/heiwa_shell/tests/worker_in_worktree.rs`.** `cmd::workspace` keeps
   its integration tests inline in the same file, `run_in_prepared_workspace`
   is `pub(crate)`, and a second harness would have duplicated the repo helper.
4. **Task 1 needed a second edit the plan did not name.** The operator fold in
   `crates/heiwa_session/src/operator.rs` matches `OperatorEventType`
   exhaustively with no `_` arm, so the five new variants were a compile error
   until classified. They are nonterminal thread touches: a worker exiting does
   not end its thread, because another worker can run against the same Work.
   The exhaustive match is the reason this was caught rather than silently
   ignored.
5. **Task 1's test file is `operator_journal.rs`**, not `operator.rs`.
6. **Task 10 also needed `runs` in `foundation_b_targets`.**
   `ci_rust_test_group.sh` validates integration-test targets as well as
   package membership.
7. **Two extra tests were added beyond the plan**: a failing worker records a
   nonzero exit rather than vanishing, and a worker does not inherit the
   operator environment. The second asserts `[absent]` from inside the child,
   so removing `env_clear()` becomes a test failure rather than a credential
   leak.

Still open for A1-c3, and deliberately not decided here:

- Whether launching a worker belongs *inside* an operator turn (gaining route,
  approval, and receipt events) or stays a peer of one. A1-c2 emits no
  `turn_id` on worker events.
- Whether worker launch must pass the Action Gate. Today `heiwa work run`
  spawns whatever it is given. The spec calls raw terminal commands an explicit
  advanced capability, which implies a gate this slice does not have.
