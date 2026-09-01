# Work Fabric A1-c3 — Surface Agreement, Restart Recovery, and the A1 Gate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Home, Work, and Agent structurally incapable of disagreeing about a Work's identity, revision, and cursor; make a worker that did not survive a restart read as `stale` exactly once without repeating its effects; and land the exact-HEAD acceptance script that lets Release A1 be called complete.

**Architecture:** Three surface views become *pure functions of one `WorkSessionSnapshotV1`* in a new `heiwa_work::surface` module, so agreement is a property of construction rather than a thing to keep in sync. Restart recovery appends one idempotent `worker_stale` marker per orphaned worker and never re-spawns. `scripts/check_work_fabric_a1_acceptance.sh` drives the real installed binary end-to-end against a temporary runtime root and repository, then stamps a clean exact HEAD.

**Tech Stack:** Rust 2021, `heiwa_work`, `heiwa_worker`, `heiwa_evidence`, `heiwa_shell` CLI and app API, bash. No new external crates.

---

## Scope and release boundary

Closes ledger rows 8, 9, and 10 of Release A1-c, which is the whole of A1.

| Row | Delivers |
|---|---|
| 8 | Home/Work/Agent agree on Work, revision, epoch, and cursor |
| 9 | Restart recovery exposes stale worker and pane truth without repeating effects |
| 10 | `scripts/check_work_fabric_a1_acceptance.sh`, additive, exact-HEAD stamped |

**Decided before this plan, do not relitigate:**

1. **Row 8 is contract level only.** Prove the three surfaces consume one read
   model and cannot diverge. No new Tauri UI, no TypeScript surface work — that
   would pull `apps/heiwa_app/desktop`, vitest, and the L0 desktop acceptance
   gate into scope for no additional truth.
2. **Worker launch stays ungated.** `heiwa work run` spawns what it is given.
   The spec calls raw terminal commands an explicit advanced capability, which
   implies an Action Gate. That is deferred with reason in the ledger, named as
   a deliberate hole, and does not block A1.

## Design decisions

1. **Agreement by construction, not by test.** If `home_view`, `work_view`, and
   `agent_view` each take `&WorkSessionSnapshotV1` and copy the identity
   quadruple from it, they cannot disagree. The test proves the property; the
   type signature is what enforces it.
2. **Views select, they never compute.** A view may hide collections and reshape
   rows for display. It may not recompute a revision, re-fold events, or reach
   for a second store. A view that needs a fact the snapshot lacks is a signal
   to add it to the projector, not to the view.
3. **`worker_stale` is its own event.** Reusing `worker_exited` with a magic
   `failure_code` would make `stale` and `failed` indistinguishable in the fold,
   and the spec lists them as separate states a UI must distinguish.
4. **Recovery is idempotent and effect-free.** It appends one marker per
   orphaned worker and never spawns, re-runs, or touches a worktree. Running it
   twice appends once.
5. **The acceptance script drives the real binary.** A gate that calls library
   functions proves the library. A1's claim is that a *user* can do this, so the
   gate runs `heiwa` as a process against a temp `HEIWA_HOME`.

## Existing substrate

- `crates/heiwa_work/src/snapshot.rs:56` — `WorkSessionSnapshotV1` with
  `work_id`, `work_revision`, `projection_epoch`, `projection_revision`,
  `operator_cursor`.
- `crates/heiwa_work/src/session.rs` — `build_work_session`, ten collections
  including `runs`.
- `crates/heiwa_worker/src/projector.rs` — `fold_runs`, `RunRow`.
- `crates/heiwa_worker/src/model.rs` — `WorkerState::Stale`, `PaneState::Stale`,
  both already defined and currently unreachable.
- `crates/heiwa_evidence/src/state.rs:97` — `recover_interrupted`, which closes
  worker sessions and revokes live leases on restart.
- `apps/heiwa_shell/src/cmd/app.rs:1706` — `is_operator_api_path`, the pattern
  new API paths follow.
- `scripts/check_l2_acceptance.sh` — the stamp-on-clean-HEAD shape to copy.

---

## Task 1: Surface views over one snapshot

**Files:**
- Create: `crates/heiwa_work/src/surface.rs`
- Modify: `crates/heiwa_work/src/lib.rs`
- Test: `crates/heiwa_work/tests/surface_agreement.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/heiwa_work/tests/surface_agreement.rs`. Reuse the row helpers
from `work_session.rs` by copying `event()` and `rows()` into this file — the
existing tests keep their helpers private per file, and a shared test-support
crate is not worth adding for two files.

```rust
#[test]
fn all_three_surfaces_report_the_same_identity_revision_and_cursor() {
    let snapshot = build_work_session(&rows(), "work-abc", WorkSessionBuildOptions::new("seed", 50))
        .expect("snapshot");

    let home = home_view(&snapshot);
    let work = work_view(&snapshot);
    let agent = agent_view(&snapshot);

    for view in [&home.identity, &work.identity, &agent.identity] {
        assert_eq!(view.work_id, snapshot.work_id);
        assert_eq!(view.work_revision, snapshot.work_revision);
        assert_eq!(view.projection_epoch, snapshot.projection_epoch);
        assert_eq!(view.projection_revision, snapshot.projection_revision);
        assert_eq!(view.operator_cursor, snapshot.operator_cursor);
    }
    assert_eq!(home.identity, work.identity);
    assert_eq!(work.identity, agent.identity);
}

#[test]
fn each_surface_selects_the_collections_its_role_answers_for() {
    let snapshot = build_work_session(&rows(), "work-abc", WorkSessionBuildOptions::new("seed", 50))
        .expect("snapshot");

    // Home answers what needs the user and what is working.
    assert!(home_view(&snapshot).collections.contains_key("blockers"));
    assert!(home_view(&snapshot).collections.contains_key("approvals"));
    // Agent answers what is running, so runs are mandatory there.
    assert!(agent_view(&snapshot).collections.contains_key("runs"));
    // Work answers the objective and its evidence.
    assert!(work_view(&snapshot).collections.contains_key("artifacts"));
}

#[test]
fn a_surface_never_invents_a_collection_the_snapshot_does_not_have() {
    let snapshot = build_work_session(&rows(), "work-abc", WorkSessionBuildOptions::new("seed", 50))
        .expect("snapshot");
    for view in [home_view(&snapshot), work_view(&snapshot), agent_view(&snapshot)] {
        for name in view.collections.keys() {
            assert!(
                snapshot.collections.contains_key(name),
                "{name} is not in the snapshot; a view selects, it never computes"
            );
        }
    }
}

#[test]
fn truncation_is_carried_through_so_a_surface_cannot_render_a_bound_as_completeness() {
    let mut rows = rows();
    for index in 0..5 {
        rows.push(artifact_row(index));
    }
    let snapshot = build_work_session(&rows, "work-abc", WorkSessionBuildOptions::new("seed", 2))
        .expect("snapshot");
    let work = work_view(&snapshot);
    assert!(
        work.truncated_collections.contains_key("artifacts"),
        "a bounded collection must stay visibly bounded on every surface"
    );
}
```

Write `artifact_row(index)` as a concrete helper producing an
`OperatorEventType::ArtifactCreated` `CursorEvent` with a unique
`payload["artifact_id"]`, mirroring the pattern already in
`work_session.rs::rows_with_a_run`.

- [ ] **Step 2: Run and confirm it fails**

```bash
cargo test -p heiwa_work --test surface_agreement -- --nocapture
```

Expected: compile error, `cannot find function home_view`.

- [ ] **Step 3: Implement the views**

Create `crates/heiwa_work/src/surface.rs`:

```rust
//! Home, Work, and Agent as pure selections over one Work-session snapshot.
//!
//! The spec requires the three surfaces never to disagree about what is
//! running, blocked, approved, changed, or complete. That is enforced here by
//! construction: each view takes `&WorkSessionSnapshotV1` and copies the
//! identity quadruple from it, so there is no second place for a revision or a
//! cursor to come from. A view selects and reshapes; it never re-folds.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::snapshot::{CollectionRows, ProjectionEpoch, WorkSessionSnapshotV1};

/// The quadruple every surface must agree on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceIdentity {
    pub work_id: String,
    pub work_revision: u64,
    pub projection_epoch: ProjectionEpoch,
    pub projection_revision: u64,
    pub operator_cursor: Option<String>,
}

impl SurfaceIdentity {
    fn of(snapshot: &WorkSessionSnapshotV1) -> Self {
        Self {
            work_id: snapshot.work_id.clone(),
            work_revision: snapshot.work_revision,
            projection_epoch: snapshot.projection_epoch.clone(),
            projection_revision: snapshot.projection_revision,
            operator_cursor: snapshot.operator_cursor.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SurfaceView {
    pub surface: String,
    pub identity: SurfaceIdentity,
    pub collections: BTreeMap<String, CollectionRows>,
    /// Carried verbatim so a surface can never render a bound as completeness.
    pub truncated_collections: BTreeMap<String, usize>,
}

fn select(snapshot: &WorkSessionSnapshotV1, surface: &str, names: &[&str]) -> SurfaceView {
    let mut collections = BTreeMap::new();
    let mut truncated = BTreeMap::new();
    for name in names {
        if let Some(rows) = snapshot.collections.get(*name) {
            collections.insert((*name).to_string(), rows.clone());
        }
        if let Some(count) = snapshot.truncated_collections.get(*name) {
            truncated.insert((*name).to_string(), *count);
        }
    }
    SurfaceView {
        surface: surface.to_string(),
        identity: SurfaceIdentity::of(snapshot),
        collections,
        truncated_collections: truncated,
    }
}

/// Home: what needs the user, what is working, what recently completed.
pub fn home_view(snapshot: &WorkSessionSnapshotV1) -> SurfaceView {
    select(
        snapshot,
        "home",
        &["work", "blockers", "approvals", "runs", "receipts"],
    )
}

/// Work: objective, conversation, workspace, and the evidence it produced.
pub fn work_view(snapshot: &WorkSessionSnapshotV1) -> SurfaceView {
    select(
        snapshot,
        "work",
        &[
            "work",
            "threads",
            "workspace",
            "actions",
            "artifacts",
            "tests",
            "receipts",
            "blockers",
        ],
    )
}

/// Agent: workers, panes, worktrees, and the actions they took.
pub fn agent_view(snapshot: &WorkSessionSnapshotV1) -> SurfaceView {
    select(
        snapshot,
        "agent",
        &["work", "runs", "workspace", "actions", "approvals"],
    )
}

/// Every surface this build serves, for callers that enumerate rather than
/// hardcode.
pub fn view_for(snapshot: &WorkSessionSnapshotV1, surface: &str) -> Option<SurfaceView> {
    match surface {
        "home" => Some(home_view(snapshot)),
        "work" => Some(work_view(snapshot)),
        "agent" => Some(agent_view(snapshot)),
        _ => None,
    }
}
```

Add to `crates/heiwa_work/src/lib.rs`:

```rust
pub mod surface;

pub use surface::{agent_view, home_view, view_for, work_view, SurfaceIdentity, SurfaceView};
```

`ProjectionEpoch` must derive `Clone` for `SurfaceIdentity::of`. If it does not,
add `Clone` to its derive list in `snapshot.rs` — it is a value type.

- [ ] **Step 4: Run and confirm the tests pass**

```bash
cargo test -p heiwa_work --locked
```

- [ ] **Step 5: Commit**

```bash
git add crates/heiwa_work/
git commit -m "feat(work): derive Home, Work, and Agent from one Work-session snapshot"
```

---

## Task 2: Serve the surfaces from the CLI and the app API

**Files:**
- Modify: `apps/heiwa_shell/src/cmd/work.rs`
- Modify: `apps/heiwa_shell/src/cmd/app.rs`
- Test: `apps/heiwa_shell/src/cmd/work.rs` (inline `mod tests`)

- [ ] **Step 1: Write the failing test**

Add to the `mod tests` in `apps/heiwa_shell/src/cmd/work.rs`:

```rust
#[test]
fn every_surface_renders_the_same_identity_from_one_snapshot() {
    let root = root();
    let created = create(root.path(), "surface agreement", "install-1").expect("create");
    let work_id = created["work_id"].as_str().expect("work id");

    let snapshot = session(root.path(), work_id, "seed").expect("session");
    let home = heiwa_work::home_view(&snapshot);
    let work = heiwa_work::work_view(&snapshot);
    let agent = heiwa_work::agent_view(&snapshot);

    assert_eq!(home.identity, work.identity);
    assert_eq!(work.identity, agent.identity);
    assert_eq!(home.identity.work_id, work_id);
}

#[test]
fn an_unknown_surface_is_refused_rather_than_defaulted() {
    assert!(heiwa_work::view_for(
        &heiwa_work::WorkSessionSnapshotV1::default(),
        "dashboard"
    )
    .is_none());
}
```

`WorkSessionSnapshotV1` must derive `Default` for the second test; add it if
absent, alongside the derives it already carries.

- [ ] **Step 2: Run and confirm it fails**

```bash
cargo test -p heiwa-shell --bin heiwa cmd::work::tests::every_surface -- --nocapture
```

- [ ] **Step 3: Add `--surface` to `heiwa work show`**

In `show_command`, read an optional `--surface <name>` flag. When present,
resolve it with `heiwa_work::view_for` and refuse an unknown name with
`unknown surface: {name}; expected home, work, or agent`. With `--json`, print
the `SurfaceView`. Without `--json`, print the identity line followed by only
the collections that view selected, reusing the existing `runs` block for the
`agent` and `home` surfaces. With no `--surface`, behaviour is unchanged.

Update `print_help` with:

```
  heiwa work show <work-id> [--surface home|work|agent] [--json]
```

- [ ] **Step 4: Add the API path**

In `apps/heiwa_shell/src/cmd/app.rs`, serve
`GET /api/v1/work/{work_id}/session` with an optional `?surface=` query
parameter, returning the full snapshot when the parameter is absent and the
`SurfaceView` when present. Follow the existing `query_param` helper and the
`is_operator_api_path` gating pattern so the new path inherits the same
loopback and signed-request treatment as the operator API — do not add a new
authentication path.

- [ ] **Step 5: Run and confirm the tests pass**

```bash
cargo test -p heiwa-shell --bin heiwa cmd::work
cargo test -p heiwa-shell --test app_api
```

- [ ] **Step 6: Commit**

```bash
git add apps/heiwa_shell/
git commit -m "feat(shell): serve Home, Work, and Agent from the one Work-session contract"
```

---

## Task 3: `worker_stale` after a restart

**Files:**
- Modify: `crates/heiwa_evidence/src/operator.rs`
- Modify: `crates/heiwa_worker/src/events.rs`
- Modify: `crates/heiwa_worker/src/projector.rs`
- Modify: `crates/heiwa_session/src/operator.rs`
- Test: `crates/heiwa_worker/tests/runs.rs`

- [ ] **Step 1: Write the failing test**

Append to `crates/heiwa_worker/tests/runs.rs`:

```rust
#[test]
fn a_worker_marked_stale_reads_stale_not_live_and_its_pane_follows() {
    let mut next = ids();
    let id = identity("work-1", "worker-1");
    let p = pane("work-1", "worker-1", "pane-1");
    let events = vec![
        worker_launched_event(&id, "2026-08-27T00:00:00Z", &mut next),
        pane_opened_event(&p, "thread-1", "2026-08-27T00:00:00Z", &mut next),
        worker_heartbeat_event(&id, 4242, "2026-08-27T00:00:01Z", &mut next),
        worker_stale_event(&id, "restart", "2026-08-27T01:00:00Z", &mut next),
    ];
    let runs = fold_runs(&events, "work-1");
    assert_eq!(runs[0].worker_state, WorkerState::Stale);
    assert_eq!(runs[0].pane_state, Some(PaneState::Stale));
    // Stale is not an ending: no exit code was observed.
    assert_eq!(runs[0].exit_code, None);
    assert_eq!(runs[0].failure_code.as_deref(), Some("restart"));
}

#[test]
fn an_exited_worker_is_not_downgraded_to_stale_by_a_later_marker() {
    let mut next = ids();
    let id = identity("work-1", "worker-1");
    let events = vec![
        worker_launched_event(&id, "2026-08-27T00:00:00Z", &mut next),
        worker_exited_event(&id, Some(0), None, "2026-08-27T00:00:02Z", &mut next),
        worker_stale_event(&id, "restart", "2026-08-27T01:00:00Z", &mut next),
    ];
    // A worker that already ended stays ended. Recovery must not rewrite a
    // known outcome into an unknown one.
    assert_eq!(fold_runs(&events, "work-1")[0].worker_state, WorkerState::Exited);
}
```

- [ ] **Step 2: Run and confirm it fails**

```bash
cargo test -p heiwa_worker --test runs stale -- --nocapture
```

Expected: `cannot find function worker_stale_event`.

- [ ] **Step 3: Add the event type**

In `crates/heiwa_evidence/src/operator.rs`, add `WorkerStale` after
`WorkerExited`. Then extend the exhaustive match in
`crates/heiwa_session/src/operator.rs::apply_to_existing_thread` with
`OperatorEventType::WorkerStale` in the same nonterminal-touch arm as the other
worker events, and add a round-trip case to
`crates/heiwa_evidence/tests/operator_journal.rs::worker_and_pane_event_types_round_trip_through_json`
for the wire name `worker_stale`.

- [ ] **Step 4: Add the builder and the fold arm**

In `crates/heiwa_worker/src/events.rs`:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerStalePayload {
    pub worker_id: String,
    /// Why the runtime could no longer vouch for this worker.
    pub reason: String,
}

from_event!(WorkerStalePayload, OperatorEventType::WorkerStale);

pub fn worker_stale_event(
    identity: &WorkerIdentity,
    reason: &str,
    occurred_at: &str,
    new_event_id: impl FnOnce() -> String,
) -> OperatorEvent {
    let payload = serde_json::to_value(WorkerStalePayload {
        worker_id: identity.worker_id.clone(),
        reason: reason.to_string(),
    })
    .expect("worker stale payload is plain data");
    worker_scoped(
        &identity.work_id,
        &identity.thread_id,
        &identity.worker_id,
        OperatorEventType::WorkerStale,
        occurred_at,
        payload,
        new_event_id,
    )
}
```

Export it from `lib.rs` alongside the other builders.

In `crates/heiwa_worker/src/projector.rs`, add the arm:

```rust
            OperatorEventType::WorkerStale => {
                let Some(id) = event.run_id.as_deref() else {
                    continue;
                };
                let Some(row) = rows.get_mut(id) else {
                    continue;
                };
                // A known ending outranks an unknown one. Recovery marks what
                // it could not vouch for; it does not rewrite what it can.
                if matches!(row.worker_state, WorkerState::Exited | WorkerState::Failed) {
                    continue;
                }
                row.worker_state = WorkerState::Stale;
                if let Some(payload) = WorkerStalePayload::from_event(event) {
                    row.failure_code = Some(payload.reason);
                }
                if row.pane_id.is_some() {
                    row.pane_state = Some(PaneState::for_worker(WorkerState::Stale));
                }
            }
```

- [ ] **Step 5: Run and confirm the tests pass**

```bash
cargo test -p heiwa_worker -p heiwa_evidence -p heiwa-session --locked
```

- [ ] **Step 6: Commit**

```bash
git add crates/
git commit -m "feat(worker): mark a worker the runtime can no longer vouch for as stale"
```

---

## Task 4: Recovery appends the marker once and spawns nothing

**Files:**
- Create: `apps/heiwa_shell/src/cmd/recover.rs`
- Modify: `apps/heiwa_shell/src/cmd/mod.rs`
- Modify: `apps/heiwa_shell/src/main.rs`
- Test: `apps/heiwa_shell/src/cmd/recover.rs` (inline `mod tests`)

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn recovery_marks_an_orphaned_worker_stale_exactly_once() {
    let source = repo();
    let runtime = tempfile::tempdir().expect("runtime");
    let (evidence, work_id) = prepared_work(runtime.path(), source.path());
    // A worker that launched and heartbeat but never exited: a crash.
    launch_without_exit(&evidence, &work_id);

    let first = recover_orphaned_workers(&evidence).expect("first recovery");
    assert_eq!(first.marked_stale, 1);

    let second = recover_orphaned_workers(&evidence).expect("second recovery");
    assert_eq!(second.marked_stale, 0, "recovery must be idempotent");

    let runs = heiwa_worker::fold_runs(&replay(&evidence, &work_id), &work_id);
    assert_eq!(runs[0].worker_state, heiwa_worker::WorkerState::Stale);
}

#[test]
fn recovery_leaves_a_completed_run_alone() {
    let source = repo();
    let runtime = tempfile::tempdir().expect("runtime");
    let (evidence, work_id) = prepared_work(runtime.path(), source.path());
    run_in_prepared_workspace(
        &evidence, &work_id, "install-1", "local",
        &["/bin/sh".into(), "-c".into(), "true".into()],
    )
    .expect("run");

    let report = recover_orphaned_workers(&evidence).expect("recovery");
    assert_eq!(report.marked_stale, 0);
    let runs = heiwa_worker::fold_runs(&replay(&evidence, &work_id), &work_id);
    assert_eq!(runs[0].worker_state, heiwa_worker::WorkerState::Exited);
}

#[test]
fn recovery_does_not_touch_the_worktree_or_start_anything() {
    let source = repo();
    let runtime = tempfile::tempdir().expect("runtime");
    let (evidence, work_id) = prepared_work(runtime.path(), source.path());
    launch_without_exit(&evidence, &work_id);

    let worktree = runtime.path().join("worktrees").join(&work_id);
    let before = std::fs::read_dir(&worktree).expect("worktree").count();
    recover_orphaned_workers(&evidence).expect("recovery");
    let after = std::fs::read_dir(&worktree).expect("worktree").count();
    assert_eq!(before, after, "recovery records, it never re-runs");
}
```

Write `launch_without_exit` concretely: build a `WorkerIdentity` the way
`run_in_prepared_workspace` does from the `workspace_prepared` payload, then
append `worker_launched_event` and `worker_heartbeat_event` through
`OperatorSessionService` and stop. Reuse `repo`, `prepared_work`, and `replay`
from `cmd::worker::tests` by making them `pub(crate)` in a shared
`#[cfg(test)] pub(crate) mod fixtures` inside `cmd/worker.rs` rather than
copying them.

- [ ] **Step 2: Run and confirm it fails**

```bash
cargo test -p heiwa-shell --bin heiwa cmd::recover -- --nocapture
```

- [ ] **Step 3: Implement recovery**

Create `apps/heiwa_shell/src/cmd/recover.rs` with:

```rust
pub struct RecoveryReport {
    pub marked_stale: usize,
}

/// Mark every worker the runtime can no longer vouch for.
///
/// A worker with a launch and no ending did not survive whatever ended the
/// previous process. This records that and nothing else: it does not re-spawn,
/// re-run, reattach, or touch a worktree, because the spec forbids silently
/// resurrecting or repeating risky work.
pub(crate) fn recover_orphaned_workers(evidence_root: &Path) -> Result<RecoveryReport>
```

Implementation: replay the whole operator journal once. For every `work_id`
present, `fold_runs` it. For each row whose `worker_state` is `Starting` or
`Live`, reconstruct the minimal `WorkerIdentity` needed by the builder from the
row's own fields and append one `worker_stale_event` with reason `restart`.
Rows already `Stale`, `Exited`, `Failed`, or `Revoked` are skipped, which is
what makes a second run append nothing.

Wire it as `heiwa work recover [--json]` in `cmd/work.rs::run` and add it to
`print_help`. Do **not** call it implicitly from other commands in this slice:
an implicit append on every CLI invocation is a side effect the user did not
ask for, and A1 has no supervisor process to own it.

- [ ] **Step 4: Run and confirm the tests pass**

```bash
cargo test -p heiwa-shell --bin heiwa cmd::recover
cargo test -p heiwa-shell --bin heiwa cmd::worker
```

- [ ] **Step 5: Commit**

```bash
git add apps/heiwa_shell/
git commit -m "feat(shell): recover orphaned workers as stale without repeating effects"
```

---

## Task 5: The A1 acceptance gate

**Files:**
- Create: `scripts/check_work_fabric_a1_acceptance.sh`
- Modify: `docs/superpowers/ledgers/2026-08-22-work-fabric-task-ledger.md`

- [ ] **Step 1: Write the script**

Create `scripts/check_work_fabric_a1_acceptance.sh`, modelled on
`scripts/check_l2_acceptance.sh`. It must be local-only, use no network, and
prove the following against a temporary `HEIWA_HOME` and a temporary git
repository, using the **built binary**, not `cargo test`:

```
1.  heiwa setup            creates an identity
2.  heiwa work create      returns a work_id
3.  heiwa workspace prepare  creates a worktree on heiwa/<work_id> and reports
                             a worker_id and lease_id
4.  heiwa work run         runs in that worktree:
      - the child's `pwd -P` equals the reported worktree path
      - a file the child creates exists in the worktree
      - the SOURCE repository is still clean
      - an exported HEIWA_ACCEPT_FAKE_SECRET reads as absent in the child
5.  heiwa work show --json contains exactly one runs row, worker_state
                          "exited", exit_code 0, and a 64-char
                          executable_sha256
6.  heiwa work recover     marks nothing (the run completed), and a second
                          invocation is also a no-op
7.  heiwa work show --surface home|work|agent  all report the same work_id,
                          work_revision, projection_epoch, projection_revision,
                          and operator_cursor
8.  the ledger's A1 rows are all `done`  (fail if a row is still pending while
                          this script is being asked to certify completion)
```

Then, and only on a clean tree at an exact HEAD:

```bash
if [[ -z "$(git status --porcelain)" ]]; then
  mkdir -p .claude && git rev-parse HEAD > .claude/work-fabric-a1-accept-sha
  printf 'Work Fabric A1 acceptance gate passed (stamp written for HEAD).\n'
else
  printf 'Work Fabric A1 acceptance gate passed. Tree is dirty, so no HEAD stamp was written.\n'
fi
```

Build the binary once at the top with
`cargo build --locked -p heiwa-shell --bin heiwa` and use
`target/debug/heiwa`. Clean the temporary directories with a `trap`.

- [ ] **Step 2: Run it and confirm it passes**

```bash
bash scripts/check_work_fabric_a1_acceptance.sh
```

Expected: every numbered check reports OK and the stamp line prints. If any
check fails, fix the runtime — do not weaken the check.

- [ ] **Step 3: Confirm the Stop gate now recognises it**

The gate already looks for this exact path. Move the ledger's rows 8, 9, and 10
to `done` with their verification commands, then:

```bash
bash scripts/hooks/stop_ledger_gate.sh
```

Expected: silent. If it blocks, the stamp is stale — re-run the acceptance
script on a clean tree.

- [ ] **Step 4: Confirm `check_ci_local.sh` picks it up**

```bash
HEIWA_BRANCH_MODE=experimental bash scripts/check_ci_local.sh 2>&1 | grep work_fabric
```

Expected: `check_work_fabric_a1_acceptance    OK` — it was reporting
`SKIP (not yet written)` until this task.

- [ ] **Step 5: Update the ledger and commit**

Move rows 8-10 to `done`. Update the A1-c preamble to say A1 is complete and
verified by its script. Replace "Next experimental slice" with Release A2.
Keep every entry in "Deferred with reason", and add the ungated worker launch
as its own bullet naming the slice that will close it.

```bash
git add scripts/check_work_fabric_a1_acceptance.sh docs/superpowers/ledgers/2026-08-22-work-fabric-task-ledger.md
git commit -m "feat(work): add the exact-HEAD Work Fabric A1 acceptance gate"
```

---

## Self-review notes

- **Row 8** is satisfied by construction (Task 1) and served on both surfaces
  that exist today, CLI and app API (Task 2). No desktop UI, by decision.
- **Row 9** is satisfied by Tasks 3 and 4. "Without repeating effects" is tested
  three ways: idempotence, a completed run left alone, and an untouched
  worktree.
- **Row 10** is Task 5, and it certifies the user-visible chain rather than the
  library, which is what the A1 claim actually is.
- **Honesty check.** After this slice A1 is complete *as scoped*. It is not the
  spec's full integrated acceptance: that needs two repositories (A2), GitHub
  publication (B), and a productivity ecosystem (C). The ledger must not imply
  otherwise.
- **Type consistency.** `SurfaceIdentity`, `SurfaceView`, `home_view`,
  `work_view`, `agent_view`, `view_for`, `worker_stale_event`,
  `WorkerStalePayload`, `recover_orphaned_workers`, `RecoveryReport` are spelled
  identically in every task that names them.
