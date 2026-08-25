# Work Fabric A1-a — Durable Work Core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `Work` a durable, replayable aggregate keyed by `work_id`, produced through the existing single operator-domain writer, and deliver it to clients as a bounded snapshot plus epoch-guarded typed deltas.

**Architecture:** `Work` is not a new store. It is a fold over operator-domain events that `OperatorSessionService` already owns and appends. A new I/O-free crate `heiwa_work` defines the aggregate, the migration resolver, and the snapshot/delta contracts; the shell wires them to the resolved configuration root and exposes `heiwa work`. No second write authority is introduced, and no historical row is rewritten.

**Tech Stack:** Rust 2021, `heiwa_evidence` operator journal (append-only JSONL with opaque cursors), `serde`, `uuid`, `sha2`. No new external dependencies beyond what the workspace already uses.

---

## Scope

This is **plan 1 of 3** for Release A1. The spec's A1 covers durable Work, `work_id`
migration, snapshot/delta delivery, one repository, one worktree, one worker and
pane, tri-surface agreement, diff/test/artifact/approval/receipt, and restart
recovery. That is several independently valuable subsystems, so it is split:

| Plan | Delivers | Independently useful because |
|---|---|---|
| **A1-a (this plan)** | Durable `Work`, canonical `work_id`, append-only migration, projector, bounded snapshot, epoch-guarded deltas, `heiwa work` CLI | A user can create, list, inspect, and replay durable Work, and a client can hold a correct incremental projection of it |
| A1-b | Workspace Coordinator: one repository, one isolated worktree, writer lease, dirty-tree preservation, diff and test projections | Governed single-repository execution against real files |
| A1-c | Worker + terminal pane bound to Work, tri-surface agreement, approval → receipt through Action Gate, restart recovery | The complete A1 loop, and the gate `scripts/check_work_fabric_a1_acceptance.sh` |

`scripts/check_work_fabric_a1_acceptance.sh` is written in A1-c, when the whole
checkpoint can actually pass. A1-a adds ledger rows only.

**Deferred with reason, not omitted:** `work_node_bound` and
`prior_history_digest` (spec WF-R15) are not built here. They require an
enrolled mesh node, and the attested-prefix design exists precisely so binding
adds a later event without changing any earlier one. Building the event now
would produce a type nothing can emit. It lands with mesh binding work.

## Existing substrate this plan builds on

Read these before starting. Every fact below was verified at `427ba193`.

- `crates/heiwa_evidence/src/operator.rs` — `OperatorEvent` (line 130),
  `OperatorEventType` (line 101, a **closed** enum), `CursorEvent` (line 164),
  `OperatorPage`, `OPERATOR_EVENT_SCHEMA_VERSION = 1` (line 39). A line whose
  `event_type` does not deserialize is reported as `None` and counted in
  `skipped_lines` — it never panics.
- `crates/heiwa_session/src/operator.rs` — `OperatorSessionService`:
  `ensure_thread` (415), `append_event` (580), `events_after` (596),
  `thread` (729), `list_threads` (765), `validate_event` (1023).
  `append_event` takes a writer lock, folds the projection, validates, appends.
- `OperatorActor { kind, id }`, `OperatorRisk::{Low,Medium,High,Critical}`,
  `OperatorSensitivity::{PublicSafe,LocalPrivate,Restricted}`.
- `heiwa_evidence`'s `operator` module is **private**. Every operator type is
  reached through the crate root — `heiwa_evidence::OperatorEvent`, not
  `heiwa_evidence::operator::OperatorEvent`. The re-export list is at
  `crates/heiwa_evidence/src/lib.rs:35`.
- `crates/heiwa_config::HeiwaPaths` is the only per-user root resolver.
  `apps/heiwa_shell/src/home.rs::heiwa_runtime_dir()` is the shell's single call
  into it. `scripts/check_l0_acceptance.sh` fails on a second resolver.
- `crates/heiwa_mesh/` is the precedent for a new crate: `*_in(&Path, …)` pure
  functions, no root resolution inside the crate, injected side effects.

## File Structure

**Create:**

| Path | Responsibility |
|---|---|
| `crates/heiwa_work/Cargo.toml` | Manifest. Depends on `heiwa_evidence`, `serde`, `serde_json`, `uuid`, `thiserror`. |
| `crates/heiwa_work/src/lib.rs` | Crate docs, `WorkError`, `Result`, re-exports. |
| `crates/heiwa_work/src/model.rs` | `WorkId`, `Work`, `WorkStatus`, `SCHEMA_VERSION`. |
| `crates/heiwa_work/src/events.rs` | Typed payloads for `work_created` / `work_linked`, and their builders onto `OperatorEvent`. |
| `crates/heiwa_work/src/projector.rs` | `fold` from operator events into `Work`, plus `WorkProjection`. |
| `crates/heiwa_work/src/migration.rs` | `resolve_work_id` — adopt-before-generate, conflict detection. |
| `crates/heiwa_work/src/snapshot.rs` | `ProjectionEpoch`, `WorkSessionSnapshotV1`, `WorkSessionDeltaV1`, `DeltaApplyOutcome`. |
| `crates/heiwa_work/tests/work_core.rs` | Integration tests across the crate's public surface. |
| `apps/heiwa_shell/src/cmd/work.rs` | `heiwa work create|list|show` against the resolved root. |
| `docs/superpowers/ledgers/2026-08-22-work-fabric-task-ledger.md` | Ledger for the Work Fabric program. |

**Modify:**

| Path | Change |
|---|---|
| `crates/heiwa_evidence/src/operator.rs` | Add `work_id: Option<String>` to `OperatorEvent`; add `WorkCreated`/`WorkLinked` to `OperatorEventType`. |
| `crates/heiwa_session/src/operator.rs` | `validate_event` requires `work_id` on Work event types. |
| `Cargo.toml` | Add `crates/heiwa_work` to workspace members. |
| `apps/heiwa_shell/Cargo.toml` | Add `heiwa_work` dependency. |
| `apps/heiwa_shell/src/cmd/mod.rs`, `src/cli.rs`, `src/main.rs` | Register and document the `work` command. |
| `scripts/ci_rust_test_group.sh` | Add package `heiwa_work` to `foundation_packages`, target `work_core` to `foundation_b_targets`. |

**Mechanical fan-out — do not skip:** adding a field to `OperatorEvent` breaks
every struct literal. There are **19** across 8 files:
`crates/heiwa_evidence/tests/operator_journal.rs`,
`crates/heiwa_evidence/src/operator.rs`,
`crates/heiwa_session/tests/operator_service.rs`,
`crates/heiwa_session/src/operator.rs`, `crates/heiwa_session/src/lib.rs`,
`apps/heiwa_shell/src/operator.rs`, `apps/heiwa_shell/src/model_calls.rs`,
`apps/heiwa_shell/src/main.rs`. The compiler names every one. Task 1 fixes them
all with `work_id: None`.

---

### Task 1: Carry `work_id` on the operator event

**Files:**
- Modify: `crates/heiwa_evidence/src/operator.rs:130-147`
- Test: `crates/heiwa_evidence/src/operator.rs` (unit tests at the bottom of the file)

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block at the bottom of
`crates/heiwa_evidence/src/operator.rs`:

```rust
#[test]
fn an_event_without_a_work_id_round_trips_and_omits_the_field() {
    let event = test_event("evt-1");
    let json = serde_json::to_value(&event).expect("serialize");
    assert!(
        json.get("work_id").is_none(),
        "an unscoped event must not carry a null work_id: {json}"
    );
    let restored: OperatorEvent = serde_json::from_value(json).expect("deserialize");
    assert_eq!(restored.work_id, None);
}

#[test]
fn an_event_written_before_work_existed_still_reads() {
    // Every event already on disk lacks the field entirely.
    let mut json = serde_json::to_value(test_event("evt-2")).expect("serialize");
    json.as_object_mut().expect("object").remove("work_id");
    let restored: OperatorEvent = serde_json::from_value(json).expect("deserialize");
    assert_eq!(restored.work_id, None, "absence must not be an error");
}

#[test]
fn a_work_scoped_event_carries_its_work_id() {
    let mut event = test_event("evt-3");
    event.work_id = Some("work-abc".to_string());
    let json = serde_json::to_value(&event).expect("serialize");
    assert_eq!(json["work_id"], "work-abc");
    let restored: OperatorEvent = serde_json::from_value(json).expect("deserialize");
    assert_eq!(restored.work_id.as_deref(), Some("work-abc"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p heiwa_evidence --lib operator::tests::a_work_scoped_event_carries_its_work_id`
Expected: FAIL to compile — `no field 'work_id' on type 'OperatorEvent'`.

- [ ] **Step 3: Add the field**

In `crates/heiwa_evidence/src/operator.rs`, add to `OperatorEvent` immediately
after `pub call_id: Option<String>,`:

```rust
    /// The `Work` this event belongs to, when it belongs to one.
    ///
    /// Optional by design rather than by omission: events that describe a user
    /// outcome carry it, and system-wide events (capability, peer health) have
    /// no Work to name. Skipped when absent so every event already on disk
    /// deserializes unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_id: Option<String>,
```

- [ ] **Step 4: Fix all 19 construction sites**

Run `cargo build --workspace --exclude heiwa-desktop 2>&1 | grep "missing field"`
and add `work_id: None,` to each literal the compiler names. Files affected are
listed under **Mechanical fan-out** above. Do not guess a value: every existing
site predates Work and is genuinely unscoped.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p heiwa_evidence -p heiwa_session`
Expected: PASS, including the three new tests.

- [ ] **Step 6: Commit**

```bash
git add crates/heiwa_evidence crates/heiwa_session apps/heiwa_shell/src
git commit -m "feat(evidence): carry an optional work_id on operator events"
```

---

### Task 2: Add the two Work event types and require their scope

**Files:**
- Modify: `crates/heiwa_evidence/src/operator.rs:101-123`
- Modify: `crates/heiwa_session/src/operator.rs:1023` (`validate_event`)
- Test: `crates/heiwa_session/src/operator.rs` unit tests

- [ ] **Step 1: Write the failing test**

Add to the `#[cfg(test)] mod tests` block in `crates/heiwa_session/src/operator.rs`:

```rust
#[test]
fn a_work_event_without_a_work_id_is_rejected() {
    let threads = HashMap::new();
    let mut event = OperatorEvent {
        schema_version: OPERATOR_EVENT_SCHEMA_VERSION,
        event_id: "evt-1".to_string(),
        thread_id: "thread-1".to_string(),
        turn_id: None,
        run_id: None,
        call_id: None,
        work_id: None,
        event_type: OperatorEventType::WorkCreated,
        occurred_at: "2026-08-22T00:00:00Z".to_string(),
        actor: OperatorActor { kind: "user".to_string(), id: "local".to_string() },
        risk_class: OperatorRisk::Low,
        sensitivity: OperatorSensitivity::LocalPrivate,
        parent_event_id: None,
        correlation_id: None,
        source_refs: Vec::new(),
        evidence_refs: Vec::new(),
        payload: serde_json::json!({}),
    };

    let error = validate_event(&threads, &event).expect_err("work events must be scoped");
    assert!(
        error.to_string().contains("requires work_id"),
        "the refusal must name the missing field: {error}"
    );

    event.work_id = Some("work-abc".to_string());
    validate_event(&threads, &event).expect("a scoped work event is accepted");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p heiwa-session --lib a_work_event_without_a_work_id_is_rejected`
Expected: FAIL to compile — `no variant named 'WorkCreated'`.

- [ ] **Step 3: Add the variants and the validation rule**

In `crates/heiwa_evidence/src/operator.rs`, add to `OperatorEventType` after
`LegacySessionImported`:

```rust
    WorkCreated,
    WorkLinked,
```

In `crates/heiwa_session/src/operator.rs`, add beside `requires_turn_id`:

```rust
/// Event types that describe a `Work` and are meaningless without naming it.
fn requires_work_id(event_type: &OperatorEventType) -> bool {
    matches!(
        event_type,
        OperatorEventType::WorkCreated | OperatorEventType::WorkLinked
    )
}
```

and inside `validate_event`, after the `requires_call_id` block:

```rust
    if requires_work_id(&event.event_type) && event.work_id.is_none() {
        bail!(
            "rejected operator event {}: event type {:?} requires work_id",
            event.event_id,
            event.event_type
        );
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p heiwa-session --lib a_work_event_without_a_work_id_is_rejected`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/heiwa_evidence/src/operator.rs crates/heiwa_session/src/operator.rs
git commit -m "feat(operator): add work_created and work_linked event types"
```

---

### Task 3: Create the `heiwa_work` crate with the aggregate

**Files:**
- Create: `crates/heiwa_work/Cargo.toml`
- Create: `crates/heiwa_work/src/lib.rs`
- Create: `crates/heiwa_work/src/model.rs`
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Write the failing test**

Create `crates/heiwa_work/src/model.rs` containing only this test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_work_id_is_prefixed_so_it_is_never_mistaken_for_a_thread_id() {
        let id = WorkId::generate(|| "0192ac31-1f4e-7c9a-9d2b-5f6a7b8c9d0e".to_string());
        assert!(id.as_str().starts_with("work-"), "{id}");
        assert_eq!(id.as_str(), "work-0192ac31-1f4e-7c9a-9d2b-5f6a7b8c9d0e");
    }

    #[test]
    fn a_work_id_round_trips_as_a_bare_string() {
        let id = WorkId::generate(|| "abc".to_string());
        let json = serde_json::to_value(&id).expect("serialize");
        assert_eq!(json, "work-abc");
        let restored: WorkId = serde_json::from_value(json).expect("deserialize");
        assert_eq!(restored, id);
    }

    #[test]
    fn a_parsed_work_id_refuses_an_unprefixed_string() {
        assert!(WorkId::parse("work-abc").is_some());
        assert!(
            WorkId::parse("thread-abc").is_none(),
            "a thread id must never silently become a work id"
        );
        assert!(WorkId::parse("").is_none());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Create the manifest first so the crate exists:

`crates/heiwa_work/Cargo.toml`:

```toml
[package]
name = "heiwa_work"
version = "0.1.0"
edition = "2021"
license.workspace = true
repository.workspace = true
homepage.workspace = true
documentation.workspace = true
readme.workspace = true
keywords.workspace = true
categories.workspace = true
description = "Durable Work aggregate, migration, and read-model projection for Heiwa."

[dependencies]
heiwa_evidence = { path = "../heiwa_evidence" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
thiserror = "2"
uuid = { version = "1", features = ["v4"] }
```

`crates/heiwa_work/src/lib.rs`:

```rust
//! Durable `Work` — the coordination aggregate above threads and tasks.
//!
//! `Work` is a fold over operator-domain events, not a second store. The
//! runtime appends those events through `OperatorSessionService`, which stays
//! the only local domain writer; this crate is I/O-free and takes events as
//! input. See `docs/superpowers/specs/2026-08-22-heiwa-work-fabric-design.md`.

pub mod model;

pub use model::{Work, WorkId, WorkStatus, SCHEMA_VERSION};

#[derive(Debug, thiserror::Error)]
pub enum WorkError {
    #[error("work record is schema version {0}, newer than this build understands")]
    UnknownVersion(u32),
    #[error("{0}")]
    Malformed(String),
}

pub type Result<T> = std::result::Result<T, WorkError>;
```

Add `"crates/heiwa_work",` to the `members` list in the root `Cargo.toml`,
between `"crates/heiwa_mesh",` and `"crates/heiwa_oauth",`.

Run: `cargo test -p heiwa_work`
Expected: FAIL to compile — `cannot find type 'WorkId'`.

- [ ] **Step 3: Write the model**

Prepend to `crates/heiwa_work/src/model.rs`, above the test module:

```rust
//! The aggregate itself.

use serde::{Deserialize, Serialize};

/// Schema version of the folded aggregate.
pub const SCHEMA_VERSION: u32 = 1;

const WORK_ID_PREFIX: &str = "work-";

/// A Work's stable primary identity, across threads, tasks, repositories,
/// provider sessions, nodes, and surfaces.
///
/// Prefixed so a journal line, a receipt, or a mesh envelope says what kind of
/// id it is holding. A thread id must never be accepted here: threads attach
/// to Work, they do not name it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkId(String);

impl WorkId {
    /// Mint a new id. The uuid source is injected so tests are reproducible.
    pub fn generate(new_uuid: impl FnOnce() -> String) -> Self {
        Self(format!("{WORK_ID_PREFIX}{}", new_uuid()))
    }

    /// Accept an existing id, or refuse it.
    pub fn parse(value: &str) -> Option<Self> {
        let rest = value.strip_prefix(WORK_ID_PREFIX)?;
        if rest.is_empty() {
            return None;
        }
        Some(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for WorkId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Where a Work stands. Distinct from a Work Session's rendered phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkStatus {
    Active,
    Blocked,
    Cancelled,
    Failed,
    Complete,
}

/// The durable coordination aggregate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Work {
    pub schema_version: u32,
    pub work_id: WorkId,
    /// Monotonic. Every mutating command supplies its expected revision, so a
    /// stale writer reloads or replans rather than overwriting.
    pub revision: u64,
    pub intent: String,
    pub status: WorkStatus,
    /// Bound to the installation before any node key exists. Work with no
    /// `origin_node` is local-only and refused at the replication boundary.
    pub origin_installation_id: String,
    pub origin_node: Option<String>,
    pub coordinator_node: Option<String>,
    /// V1 creates exactly one thread atomically with the Work.
    pub primary_thread_id: String,
    /// Review, handoff, or channel threads added later without changing
    /// `work_id`.
    pub related_thread_ids: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
}

impl Work {
    /// Whether this Work may cross the mesh replication boundary.
    pub fn is_replicable(&self) -> bool {
        self.origin_node.is_some()
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p heiwa_work`
Expected: PASS, 3 tests.

- [ ] **Step 5: Add a replicability test and commit**

Add to the test module:

```rust
    fn work() -> Work {
        Work {
            schema_version: SCHEMA_VERSION,
            work_id: WorkId::generate(|| "abc".to_string()),
            revision: 1,
            intent: "prepare the release".to_string(),
            status: WorkStatus::Active,
            origin_installation_id: "installation-1".to_string(),
            origin_node: None,
            coordinator_node: None,
            primary_thread_id: "thread-1".to_string(),
            related_thread_ids: Vec::new(),
            created_at: "2026-08-22T00:00:00Z".to_string(),
            updated_at: "2026-08-22T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn unbound_work_is_not_replicable() {
        assert!(
            !work().is_replicable(),
            "work created before enrolment must never cross the mesh boundary"
        );
    }

    #[test]
    fn node_bound_work_is_replicable() {
        let mut bound = work();
        bound.origin_node = Some("sha256:ff".to_string());
        assert!(bound.is_replicable());
    }
```

Run: `cargo test -p heiwa_work`
Expected: PASS, 5 tests.

```bash
git add crates/heiwa_work Cargo.toml Cargo.lock
git commit -m "feat(work): add the durable Work aggregate and its identity"
```

---

### Task 4: Build and read Work events

**Files:**
- Create: `crates/heiwa_work/src/events.rs`
- Modify: `crates/heiwa_work/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/heiwa_work/src/events.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_created_event_names_its_work_in_the_envelope_not_only_the_payload() {
        let work_id = WorkId::generate(|| "abc".to_string());
        let event = work_created_event(
            &work_id,
            "thread-1",
            "prepare the release",
            "installation-1",
            "2026-08-22T00:00:00Z",
            || "evt-1".to_string(),
        );

        assert_eq!(event.work_id.as_deref(), Some("work-abc"));
        assert_eq!(event.thread_id, "thread-1");
        assert_eq!(event.event_type, OperatorEventType::WorkCreated);
        assert_eq!(event.sensitivity, OperatorSensitivity::LocalPrivate);

        let payload = WorkCreatedPayload::from_event(&event).expect("payload");
        assert_eq!(payload.intent, "prepare the release");
        assert_eq!(payload.origin_installation_id, "installation-1");
        assert_eq!(payload.primary_thread_id, "thread-1");
    }

    #[test]
    fn a_linked_event_records_whether_the_id_was_adopted_or_minted() {
        let work_id = WorkId::generate(|| "abc".to_string());
        let event = work_linked_event(
            &work_id,
            "thread-9",
            WorkLinkOrigin::Adopted,
            "2026-08-22T00:00:00Z",
            || "evt-2".to_string(),
        );

        let payload = WorkLinkedPayload::from_event(&event).expect("payload");
        assert_eq!(payload.origin, WorkLinkOrigin::Adopted);
        assert_eq!(payload.thread_id, "thread-9");
    }

    #[test]
    fn a_payload_from_the_wrong_event_type_is_refused() {
        let work_id = WorkId::generate(|| "abc".to_string());
        let created = work_created_event(
            &work_id,
            "thread-1",
            "intent",
            "installation-1",
            "2026-08-22T00:00:00Z",
            || "evt-1".to_string(),
        );
        assert!(
            WorkLinkedPayload::from_event(&created).is_none(),
            "reading a payload must check the event type, not just the shape"
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Add `pub mod events;` to `crates/heiwa_work/src/lib.rs`.

Run: `cargo test -p heiwa_work --lib events`
Expected: FAIL to compile — `cannot find function 'work_created_event'`.

- [ ] **Step 3: Write the builders and readers**

Prepend to `crates/heiwa_work/src/events.rs`:

```rust
//! Turning Work facts into operator-domain events, and back.
//!
//! The envelope carries `work_id` so a reader can scope without parsing a
//! payload; the payload carries the fields only that event type has.

use heiwa_evidence::{
    OperatorActor, OperatorEvent, OperatorEventType, OperatorRisk, OperatorSensitivity,
    OPERATOR_EVENT_SCHEMA_VERSION,
};
use serde::{Deserialize, Serialize};

use crate::model::WorkId;

/// How a thread's `work_id` was decided.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkLinkOrigin {
    /// The thread's own rows already carried one consistent, valid id.
    Adopted,
    /// No id existed anywhere in the thread's rows.
    Minted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkCreatedPayload {
    pub intent: String,
    pub origin_installation_id: String,
    pub primary_thread_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkLinkedPayload {
    pub thread_id: String,
    pub origin: WorkLinkOrigin,
}

impl WorkCreatedPayload {
    pub fn from_event(event: &OperatorEvent) -> Option<Self> {
        if event.event_type != OperatorEventType::WorkCreated {
            return None;
        }
        serde_json::from_value(event.payload.clone()).ok()
    }
}

impl WorkLinkedPayload {
    pub fn from_event(event: &OperatorEvent) -> Option<Self> {
        if event.event_type != OperatorEventType::WorkLinked {
            return None;
        }
        serde_json::from_value(event.payload.clone()).ok()
    }
}

fn local_actor() -> OperatorActor {
    OperatorActor {
        kind: "user".to_string(),
        id: "local".to_string(),
    }
}

fn scoped(
    work_id: &WorkId,
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
        work_id: Some(work_id.as_str().to_string()),
        event_type,
        occurred_at: occurred_at.to_string(),
        actor: local_actor(),
        risk_class: OperatorRisk::Low,
        sensitivity: OperatorSensitivity::LocalPrivate,
        parent_event_id: None,
        correlation_id: None,
        source_refs: Vec::new(),
        evidence_refs: Vec::new(),
        payload,
    }
}

pub fn work_created_event(
    work_id: &WorkId,
    primary_thread_id: &str,
    intent: &str,
    origin_installation_id: &str,
    occurred_at: &str,
    new_event_id: impl FnOnce() -> String,
) -> OperatorEvent {
    let payload = serde_json::json!({
        "intent": intent,
        "origin_installation_id": origin_installation_id,
        "primary_thread_id": primary_thread_id,
    });
    scoped(
        work_id,
        primary_thread_id,
        OperatorEventType::WorkCreated,
        occurred_at,
        payload,
        new_event_id,
    )
}

pub fn work_linked_event(
    work_id: &WorkId,
    thread_id: &str,
    origin: WorkLinkOrigin,
    occurred_at: &str,
    new_event_id: impl FnOnce() -> String,
) -> OperatorEvent {
    let payload = serde_json::json!({
        "thread_id": thread_id,
        "origin": origin,
    });
    scoped(
        work_id,
        thread_id,
        OperatorEventType::WorkLinked,
        occurred_at,
        payload,
        new_event_id,
    )
}
```

Add to `crates/heiwa_work/src/lib.rs` re-exports:

```rust
pub use events::{
    work_created_event, work_linked_event, WorkCreatedPayload, WorkLinkOrigin, WorkLinkedPayload,
};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p heiwa_work`
Expected: PASS, 8 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/heiwa_work
git commit -m "feat(work): build and read work_created and work_linked events"
```

---

### Task 5: Fold events into the aggregate

**Files:**
- Create: `crates/heiwa_work/src/projector.rs`
- Modify: `crates/heiwa_work/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/heiwa_work/src/projector.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::{work_created_event, work_linked_event, WorkLinkOrigin};

    fn created() -> heiwa_evidence::OperatorEvent {
        work_created_event(
            &WorkId::generate(|| "abc".to_string()),
            "thread-1",
            "prepare the release",
            "installation-1",
            "2026-08-22T00:00:00Z",
            || "evt-1".to_string(),
        )
    }

    #[test]
    fn a_created_event_folds_into_one_work_at_revision_one() {
        let projection = fold(&[created()]);
        let work = projection.work("work-abc").expect("work");

        assert_eq!(work.revision, 1);
        assert_eq!(work.intent, "prepare the release");
        assert_eq!(work.status, WorkStatus::Active);
        assert_eq!(work.primary_thread_id, "thread-1");
        assert_eq!(work.origin_installation_id, "installation-1");
        assert!(work.origin_node.is_none());
        assert_eq!(projection.skipped_events, 0);
    }

    #[test]
    fn a_linked_thread_joins_the_related_list_without_replacing_the_primary() {
        let events = vec![
            created(),
            work_linked_event(
                &WorkId::parse("work-abc").expect("id"),
                "thread-9",
                WorkLinkOrigin::Adopted,
                "2026-08-22T00:01:00Z",
                || "evt-2".to_string(),
            ),
        ];
        let projection = fold(&events);
        let work = projection.work("work-abc").expect("work");

        assert_eq!(work.primary_thread_id, "thread-1");
        assert_eq!(work.related_thread_ids, vec!["thread-9".to_string()]);
        assert_eq!(work.revision, 2, "every accepted event advances the revision");
        assert_eq!(work.updated_at, "2026-08-22T00:01:00Z");
    }

    #[test]
    fn linking_the_same_thread_twice_does_not_duplicate_it() {
        let link = |id: &str| {
            work_linked_event(
                &WorkId::parse("work-abc").expect("id"),
                "thread-9",
                WorkLinkOrigin::Adopted,
                "2026-08-22T00:01:00Z",
                || id.to_string(),
            )
        };
        let projection = fold(&[created(), link("evt-2"), link("evt-3")]);
        let work = projection.work("work-abc").expect("work");
        assert_eq!(work.related_thread_ids, vec!["thread-9".to_string()]);
    }

    #[test]
    fn an_event_for_an_unknown_work_is_counted_rather_than_inventing_one() {
        let orphan = work_linked_event(
            &WorkId::parse("work-missing").expect("id"),
            "thread-9",
            WorkLinkOrigin::Minted,
            "2026-08-22T00:01:00Z",
            || "evt-2".to_string(),
        );
        let projection = fold(&[orphan]);

        assert!(projection.work("work-missing").is_none());
        assert_eq!(
            projection.skipped_events, 1,
            "a link with no creation is damage to report, not a Work to fabricate"
        );
    }

    #[test]
    fn unrelated_operator_events_are_ignored_without_counting_as_damage() {
        let mut turn = created();
        turn.event_type = heiwa_evidence::OperatorEventType::UserMessage;
        turn.work_id = None;
        let projection = fold(&[created(), turn]);

        assert_eq!(projection.work("work-abc").expect("work").revision, 1);
        assert_eq!(
            projection.skipped_events, 0,
            "an unscoped event is not this projector's business"
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Add `pub mod projector;` to `crates/heiwa_work/src/lib.rs`.

Run: `cargo test -p heiwa_work --lib projector`
Expected: FAIL to compile — `cannot find function 'fold'`.

- [ ] **Step 3: Write the projector**

Prepend to `crates/heiwa_work/src/projector.rs`:

```rust
//! Folding operator events into Work.
//!
//! Read-only and replayable: the same events always produce the same
//! aggregate. Damage is counted rather than smoothed over, because a Work that
//! silently loses an event looks identical to one that never had it.

use std::collections::BTreeMap;

use heiwa_evidence::{OperatorEvent, OperatorEventType};

use crate::events::{WorkCreatedPayload, WorkLinkedPayload};
use crate::model::{Work, WorkId, WorkStatus, SCHEMA_VERSION};

/// Every Work visible in one stream, plus what could not be folded.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkProjection {
    works: BTreeMap<String, Work>,
    /// Work-scoped events that could not be applied: an unknown Work, a
    /// malformed payload, or a duplicate creation.
    pub skipped_events: usize,
}

impl WorkProjection {
    pub fn work(&self, work_id: &str) -> Option<&Work> {
        self.works.get(work_id)
    }

    pub fn all(&self) -> impl Iterator<Item = &Work> {
        self.works.values()
    }

    pub fn len(&self) -> usize {
        self.works.len()
    }

    pub fn is_empty(&self) -> bool {
        self.works.is_empty()
    }
}

/// Fold an ordered slice of operator events into every Work they describe.
pub fn fold(events: &[OperatorEvent]) -> WorkProjection {
    let mut projection = WorkProjection::default();

    for event in events {
        // Events with no work_id belong to some other projector.
        let Some(raw_id) = event.work_id.as_deref() else {
            continue;
        };
        let Some(work_id) = WorkId::parse(raw_id) else {
            projection.skipped_events += 1;
            continue;
        };

        match event.event_type {
            OperatorEventType::WorkCreated => {
                let Some(payload) = WorkCreatedPayload::from_event(event) else {
                    projection.skipped_events += 1;
                    continue;
                };
                if projection.works.contains_key(work_id.as_str()) {
                    // A second creation for one id is a conflict, never a
                    // reset: the first creation already owns the identity.
                    projection.skipped_events += 1;
                    continue;
                }
                projection.works.insert(
                    work_id.as_str().to_string(),
                    Work {
                        schema_version: SCHEMA_VERSION,
                        work_id,
                        revision: 1,
                        intent: payload.intent,
                        status: WorkStatus::Active,
                        origin_installation_id: payload.origin_installation_id,
                        origin_node: None,
                        coordinator_node: None,
                        primary_thread_id: payload.primary_thread_id,
                        related_thread_ids: Vec::new(),
                        created_at: event.occurred_at.clone(),
                        updated_at: event.occurred_at.clone(),
                    },
                );
            }
            OperatorEventType::WorkLinked => {
                let Some(payload) = WorkLinkedPayload::from_event(event) else {
                    projection.skipped_events += 1;
                    continue;
                };
                let Some(work) = projection.works.get_mut(work_id.as_str()) else {
                    projection.skipped_events += 1;
                    continue;
                };
                if payload.thread_id != work.primary_thread_id
                    && !work.related_thread_ids.contains(&payload.thread_id)
                {
                    work.related_thread_ids.push(payload.thread_id);
                }
                work.revision += 1;
                work.updated_at = event.occurred_at.clone();
            }
            _ => {}
        }
    }

    projection
}
```

Add to `crates/heiwa_work/src/lib.rs`:

```rust
pub mod projector;
pub use projector::{fold, WorkProjection};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p heiwa_work`
Expected: PASS, 13 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/heiwa_work
git commit -m "feat(work): fold operator events into the Work aggregate"
```

---

### Task 6: Resolve `work_id` for an existing thread — adopt before generate

**Files:**
- Create: `crates/heiwa_work/src/migration.rs`
- Modify: `crates/heiwa_work/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/heiwa_work/src/migration.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_already_on_the_threads_rows_is_adopted() {
        let outcome = resolve_work_id(
            &["work-abc".to_string(), "work-abc".to_string()],
            || "never".to_string(),
        )
        .expect("consistent rows resolve");

        assert_eq!(outcome.work_id.as_str(), "work-abc");
        assert_eq!(outcome.origin, WorkLinkOrigin::Adopted);
    }

    #[test]
    fn an_id_is_minted_only_when_no_row_carries_one() {
        let outcome = resolve_work_id(&[], || "fresh".to_string()).expect("empty resolves");

        assert_eq!(outcome.work_id.as_str(), "work-fresh");
        assert_eq!(outcome.origin, WorkLinkOrigin::Minted);
    }

    #[test]
    fn adopting_takes_precedence_over_minting() {
        // The whole point of the rule: minting here would orphan the rows
        // that already carry work-abc.
        let outcome = resolve_work_id(&["work-abc".to_string()], || {
            panic!("must not mint while an adoptable id exists")
        })
        .expect("adoptable rows resolve");
        assert_eq!(outcome.work_id.as_str(), "work-abc");
    }

    #[test]
    fn conflicting_ids_on_one_thread_are_refused_not_merged() {
        let error = resolve_work_id(
            &["work-abc".to_string(), "work-def".to_string()],
            || "fresh".to_string(),
        )
        .expect_err("two ids on one thread is a conflict");

        let MigrationConflict::AmbiguousWorkId { found } = error;
        assert_eq!(found, vec!["work-abc".to_string(), "work-def".to_string()]);
    }

    #[test]
    fn a_malformed_id_is_a_conflict_rather_than_a_reason_to_mint() {
        let error = resolve_work_id(&["thread-abc".to_string()], || "fresh".to_string())
            .expect_err("an unparseable id must not be quietly replaced");

        let MigrationConflict::AmbiguousWorkId { found } = error;
        assert_eq!(found, vec!["thread-abc".to_string()]);
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Add `pub mod migration;` to `crates/heiwa_work/src/lib.rs`.

Run: `cargo test -p heiwa_work --lib migration`
Expected: FAIL to compile — `cannot find function 'resolve_work_id'`.

- [ ] **Step 3: Write the resolver**

Prepend to `crates/heiwa_work/src/migration.rs`:

```rust
//! Deciding a thread's `work_id` when it is first promoted into Work.
//!
//! The order is normative, not advisory: minting while an adoptable id exists
//! orphans every row that already carries the old one, and nothing in the data
//! afterwards shows that it happened.

use std::collections::BTreeSet;

use crate::events::WorkLinkOrigin;
use crate::model::WorkId;

/// A thread that cannot be promoted without a human decision.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MigrationConflict {
    #[error(
        "thread carries {} distinct or unusable work ids ({}); resolve them before promoting it \
         — Heiwa will not merge them or mint a third",
        found.len(),
        found.join(", ")
    )]
    AmbiguousWorkId { found: Vec<String> },
}

/// What promoting a thread decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkIdResolution {
    pub work_id: WorkId,
    pub origin: WorkLinkOrigin,
}

/// Resolve the `work_id` for a thread from the ids its own rows already carry.
///
/// `existing_ids` is every `work_id` found on that thread's task, connector,
/// and evidence rows, in any order and with duplicates.
pub fn resolve_work_id(
    existing_ids: &[String],
    new_uuid: impl FnOnce() -> String,
) -> Result<WorkIdResolution, MigrationConflict> {
    let distinct: BTreeSet<&String> = existing_ids.iter().collect();

    // 1. Adopt.
    if !distinct.is_empty() {
        if distinct.len() == 1 {
            let only = distinct.iter().next().expect("one element");
            if let Some(work_id) = WorkId::parse(only) {
                return Ok(WorkIdResolution {
                    work_id,
                    origin: WorkLinkOrigin::Adopted,
                });
            }
        }
        return Err(MigrationConflict::AmbiguousWorkId {
            found: distinct.into_iter().cloned().collect(),
        });
    }

    // 2. Mint — only because nothing was adoptable.
    Ok(WorkIdResolution {
        work_id: WorkId::generate(new_uuid),
        origin: WorkLinkOrigin::Minted,
    })
}
```

Add to `crates/heiwa_work/src/lib.rs`:

```rust
pub use migration::{resolve_work_id, MigrationConflict, WorkIdResolution};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p heiwa_work`
Expected: PASS, 18 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/heiwa_work
git commit -m "feat(work): resolve migration work ids by adopt before generate"
```

---

### Task 7: Snapshot and epoch-guarded deltas

**Files:**
- Create: `crates/heiwa_work/src/snapshot.rs`
- Modify: `crates/heiwa_work/src/lib.rs`

- [ ] **Step 1: Write the failing test**

Create `crates/heiwa_work/src/snapshot.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn epoch(value: &str) -> ProjectionEpoch {
        ProjectionEpoch::from_seed(value)
    }

    fn client() -> ClientProjection {
        ClientProjection {
            epoch: epoch("fold-1"),
            projection_revision: 3,
        }
    }

    fn delta(base: u64, next: u64, epoch: ProjectionEpoch) -> WorkSessionDeltaV1 {
        WorkSessionDeltaV1 {
            work_id: "work-abc".to_string(),
            projection_epoch: epoch,
            base_projection_revision: base,
            projection_revision: next,
            operator_cursor: Some("cursor-9".to_string()),
            upserts: Default::default(),
            removals: Default::default(),
        }
    }

    #[test]
    fn a_delta_on_the_expected_epoch_and_revision_applies() {
        let outcome = client().accept(&delta(3, 4, epoch("fold-1")));
        assert_eq!(outcome, DeltaApplyOutcome::Applied { projection_revision: 4 });
    }

    #[test]
    fn a_revision_gap_forces_a_resync() {
        let outcome = client().accept(&delta(5, 6, epoch("fold-1")));
        assert_eq!(
            outcome,
            DeltaApplyOutcome::ResyncRequired { reason: ResyncReason::RevisionGap }
        );
    }

    #[test]
    fn a_delta_from_a_different_fold_forces_a_resync_even_at_a_matching_revision() {
        // The bug this exists to prevent: after a projector rebuild the
        // revision restarts, so base 3 matches by coincidence while the two
        // sides describe different folds.
        let outcome = client().accept(&delta(3, 4, epoch("fold-2")));
        assert_eq!(
            outcome,
            DeltaApplyOutcome::ResyncRequired { reason: ResyncReason::EpochChanged }
        );
    }

    #[test]
    fn a_replayed_delta_is_refused_rather_than_applied_twice() {
        let outcome = client().accept(&delta(2, 3, epoch("fold-1")));
        assert_eq!(
            outcome,
            DeltaApplyOutcome::ResyncRequired { reason: ResyncReason::RevisionGap }
        );
    }

    #[test]
    fn a_fresh_epoch_differs_from_the_one_before_it() {
        assert_ne!(epoch("fold-1"), epoch("fold-2"));
        assert_eq!(epoch("fold-1"), epoch("fold-1"));
    }

    #[test]
    fn a_snapshot_states_the_epoch_and_revision_a_client_must_track() {
        let snapshot = WorkSessionSnapshotV1 {
            work_id: "work-abc".to_string(),
            work_revision: 7,
            projection_epoch: epoch("fold-1"),
            projection_revision: 3,
            operator_cursor: Some("cursor-9".to_string()),
            source_watermarks: Default::default(),
            collections: Default::default(),
        };
        let resumed = ClientProjection::from_snapshot(&snapshot);
        assert_eq!(
            resumed.accept(&delta(3, 4, epoch("fold-1"))),
            DeltaApplyOutcome::Applied { projection_revision: 4 }
        );
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Add `pub mod snapshot;` to `crates/heiwa_work/src/lib.rs`.

Run: `cargo test -p heiwa_work --lib snapshot`
Expected: FAIL to compile — `cannot find type 'ProjectionEpoch'`.

- [ ] **Step 3: Write the delivery contract**

Prepend to `crates/heiwa_work/src/snapshot.rs`:

```rust
//! Bounded snapshot plus typed deltas.
//!
//! The snapshot is a baseline, not a payload resent after every event. The
//! epoch is what keeps that safe: `projection_revision` is monotonic only
//! *within* one fold and restarts whenever the projector rebuilds, so a client
//! holding revision 3 could otherwise accept a delta based on revision 3 from a
//! different fold entirely. Nothing in the resulting data would show it, so the
//! case is excluded by construction rather than detected afterwards.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Identity of one fold of the read model.
///
/// Minted on every projector build — first start, restart, upgrade, schema
/// change, compaction — and never reused across them.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProjectionEpoch(String);

impl ProjectionEpoch {
    /// Derive an epoch from whatever uniquely identifies this fold, so the
    /// value is reproducible in tests and opaque to clients.
    pub fn from_seed(seed: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(seed.as_bytes());
        let digest = hasher.finalize();
        Self(
            digest
                .iter()
                .take(8)
                .map(|byte| format!("{byte:02x}"))
                .collect(),
        )
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Rows of one collection, keyed by stable id. Bounded and paginated at the
/// delivery boundary; full logs, diffs, and bodies load through their own
/// authorized endpoints.
pub type CollectionRows = BTreeMap<String, serde_json::Value>;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkSessionSnapshotV1 {
    pub work_id: String,
    /// Durable aggregate revision.
    pub work_revision: u64,
    /// Identity of the fold this baseline came from.
    pub projection_epoch: ProjectionEpoch,
    /// Monotonic within `projection_epoch` only.
    pub projection_revision: u64,
    /// Durable operator-stream replay boundary.
    pub operator_cursor: Option<String>,
    pub source_watermarks: BTreeMap<String, String>,
    pub collections: BTreeMap<String, CollectionRows>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkSessionDeltaV1 {
    pub work_id: String,
    pub projection_epoch: ProjectionEpoch,
    pub base_projection_revision: u64,
    pub projection_revision: u64,
    pub operator_cursor: Option<String>,
    pub upserts: BTreeMap<String, CollectionRows>,
    pub removals: BTreeMap<String, Vec<String>>,
}

impl Default for ProjectionEpoch {
    fn default() -> Self {
        Self::from_seed("")
    }
}

/// Why a client must discard its projection and refetch.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResyncReason {
    /// The projector rebuilt; revisions from the old fold mean nothing now.
    EpochChanged,
    /// A delta was missed, replayed, or arrived out of order.
    RevisionGap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeltaApplyOutcome {
    Applied { projection_revision: u64 },
    ResyncRequired { reason: ResyncReason },
}

/// What a client tracks between frames.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientProjection {
    pub epoch: ProjectionEpoch,
    pub projection_revision: u64,
}

impl ClientProjection {
    pub fn from_snapshot(snapshot: &WorkSessionSnapshotV1) -> Self {
        Self {
            epoch: snapshot.projection_epoch.clone(),
            projection_revision: snapshot.projection_revision,
        }
    }

    /// Decide a delta. Epoch is checked before revision: a matching revision
    /// across two folds is a coincidence, not agreement.
    pub fn accept(&self, delta: &WorkSessionDeltaV1) -> DeltaApplyOutcome {
        if delta.projection_epoch != self.epoch {
            return DeltaApplyOutcome::ResyncRequired {
                reason: ResyncReason::EpochChanged,
            };
        }
        if delta.base_projection_revision != self.projection_revision {
            return DeltaApplyOutcome::ResyncRequired {
                reason: ResyncReason::RevisionGap,
            };
        }
        DeltaApplyOutcome::Applied {
            projection_revision: delta.projection_revision,
        }
    }
}
```

Add to `crates/heiwa_work/src/lib.rs`:

```rust
pub use snapshot::{
    ClientProjection, DeltaApplyOutcome, ProjectionEpoch, ResyncReason, WorkSessionDeltaV1,
    WorkSessionSnapshotV1,
};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p heiwa_work`
Expected: PASS, 24 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/heiwa_work
git commit -m "feat(work): guard delta delivery with a projection epoch"
```

---

### Task 8: End-to-end integration test through the real journal

**Files:**
- Create: `crates/heiwa_work/tests/work_core.rs`
- Modify: `crates/heiwa_work/Cargo.toml` (dev-dependencies)

- [ ] **Step 1: Write the failing test**

Add to `crates/heiwa_work/Cargo.toml`:

```toml
[dev-dependencies]
heiwa-session = { path = "../heiwa_session" }
tempfile = "3"
```

`heiwa_work` is a foundation package and `heiwa-session` is a runtime one, so
this edge points "upward". It is a **dev**-dependency only, which Cargo permits
even when `heiwa-session` later depends on `heiwa_work` — dev-dependency cycles
are legal and do not affect the normal build graph. Do not promote it to a
regular dependency.

Create `crates/heiwa_work/tests/work_core.rs`:

```rust
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
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p heiwa_work --test work_core`
Expected: FAIL — the tests compile only after Tasks 1–7 are complete. If they
were, expect PASS. Run this task only after those tasks are committed; a
failure here means an earlier task's contract is wrong, not this one.

- [ ] **Step 3: Fix whatever the integration reveals**

No new production code is expected. If `append_event` rejects a Work event for
a reason other than a missing `work_id`, correct the builder in
`crates/heiwa_work/src/events.rs` rather than relaxing `validate_event`.

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p heiwa_work --test work_core`
Expected: PASS, 3 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/heiwa_work Cargo.lock
git commit -m "test(work): prove Work survives the real append and replay path"
```

---

### Task 9: `heiwa work` command

**Files:**
- Create: `apps/heiwa_shell/src/cmd/work.rs`
- Modify: `apps/heiwa_shell/Cargo.toml`
- Modify: `apps/heiwa_shell/src/cmd/mod.rs`
- Modify: `apps/heiwa_shell/src/cli.rs:43`
- Modify: `apps/heiwa_shell/src/main.rs:1036`

- [ ] **Step 1: Write the failing test**

Create `apps/heiwa_shell/src/cmd/work.rs` with only this test module:

```rust
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
        let created = create(dir.path(), "prepare the release", "installation-1")
            .expect("create work");
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
}
```

- [ ] **Step 2: Run test to verify it fails**

Add to `apps/heiwa_shell/Cargo.toml`, after the `heiwa_mesh` line:

```toml
heiwa_work = { path = "../../crates/heiwa_work" }
```

Add `pub mod work;` to `apps/heiwa_shell/src/cmd/mod.rs` after `pub mod mesh;`.

Run: `cargo test -p heiwa-shell --bin heiwa cmd::work`
Expected: FAIL to compile — `cannot find function 'summarize'`.

- [ ] **Step 3: Write the command**

Prepend to `apps/heiwa_shell/src/cmd/work.rs`:

```rust
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
use heiwa_work::{fold, work_created_event, WorkId};

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
    let root = crate::home::heiwa_runtime_dir();
    let summary = summarize(&root)?;
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
    let root = crate::home::heiwa_runtime_dir();
    let identity = heiwa_identity::load_from(&root)
        .map_err(|error| anyhow!("{error}"))?
        .ok_or_else(|| {
            anyhow!("no local identity on this installation; run first-run setup before creating Work")
        })?;

    let created = create(&root, intent, &identity.installation_id)?;
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
    let service = service(root)?;
    let threads = service
        .list_threads(512)
        .map_err(|error| anyhow!("{error}"))?;

    let mut events = Vec::new();
    for thread in &threads {
        let page = service
            .events_after(&thread.thread_id, None, 1024)
            .map_err(|error| anyhow!("{error}"))?;
        events.extend(page.events.into_iter().map(|row| row.event));
    }

    let projection = fold(&events);
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

fn has_flag(args: &[String], flag: &str) -> bool {
    args.iter().any(|arg| arg == flag)
}

```

Register the command in `apps/heiwa_shell/src/cli.rs`, immediately before the
`Some("mail")` arm:

```rust
        Some("work") => {
            cmd::work::run(&args[2..])?;
            Ok(true)
        }
```

Add to `print_help()` in `apps/heiwa_shell/src/main.rs`, after the `mesh` line:

```rust
    println!("  work list|create              Durable Work on this installation");
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p heiwa-shell --bin heiwa cmd::work`
Expected: PASS, 3 tests.

- [ ] **Step 5: Commit**

```bash
git add apps/heiwa_shell Cargo.lock
git commit -m "feat(shell): add heiwa work for durable Work"
```

---

### Task 10: CI plumbing, ledger, and the full gate run

**Files:**
- Modify: `scripts/ci_rust_test_group.sh:16-31` and `:93-101`
- Create: `docs/superpowers/ledgers/2026-08-22-work-fabric-task-ledger.md`

- [ ] **Step 1: Register the crate with CI**

In `scripts/ci_rust_test_group.sh`, add `heiwa_work` to `foundation_packages`
after `heiwa_receipts`, and `work_core` to `foundation_b_targets` after
`telemetry_pane`. Keep both lists alphabetically ordered.

- [ ] **Step 2: Verify the CI grouping matches Cargo**

Run: `bash scripts/ci_rust_test_group.sh --check`
Expected: `Rust CI test groups cover every non-desktop workspace package exactly once.`

A failure here means the package or test-target name is misspelled. This
validator is the reason a new crate cannot silently compile in CI without ever
having its tests run.

- [ ] **Step 3: Write the ledger**

Create `docs/superpowers/ledgers/2026-08-22-work-fabric-task-ledger.md`:

```markdown
# Work Fabric — Task Ledger

Contract: `docs/superpowers/specs/2026-08-22-heiwa-work-fabric-design.md`
Plan: `docs/superpowers/plans/2026-08-22-work-fabric-a1a-durable-work-core.md`
Started: 2026-08-22

Status is what is true at HEAD, not what is intended. A row moves to done only
when its verification runs.

## Release A1-a — Durable Work core

| # | Step | Status | Verification |
|---|---|---|---|
| 1 | `work_id` on the operator event | todo | `cargo test -p heiwa_evidence` |
| 2 | `work_created` / `work_linked` types and scope validation | todo | `cargo test -p heiwa-session` |
| 3 | `heiwa_work` crate and the Work aggregate | todo | `cargo test -p heiwa_work` |
| 4 | Work event builders and readers | todo | `cargo test -p heiwa_work` |
| 5 | Projector fold, damage counted | todo | `cargo test -p heiwa_work` |
| 6 | Migration: adopt before generate | todo | `cargo test -p heiwa_work` |
| 7 | Snapshot and epoch-guarded deltas | todo | `cargo test -p heiwa_work` |
| 8 | Integration through the real journal | todo | `cargo test -p heiwa_work --test work_core` |
| 9 | `heiwa work` command | todo | `cargo test -p heiwa-shell --bin heiwa cmd::work` |
| 10 | CI grouping and ledger | todo | `bash scripts/ci_rust_test_group.sh --check` |

## Deferred with reason

- `work_node_bound` and `prior_history_digest` (WF-R15) need an enrolled mesh
  node. The attested-prefix design exists so binding adds a later event without
  changing an earlier one, so building the type now would produce something
  nothing can emit.
- `scripts/check_work_fabric_a1_acceptance.sh` lands in A1-c, when the whole
  A1 checkpoint can pass. A1-a alone cannot satisfy it.

## Not started

- A1-b — Workspace Coordinator: one repository, one worktree, writer lease,
  diff and test projections.
- A1-c — worker and pane bound to Work, tri-surface agreement, approval to
  receipt, restart recovery, and the A1 acceptance script.
```

- [ ] **Step 4: Run every gate CI runs**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --exclude heiwa-desktop --locked --all-targets -- -D warnings
cargo test --workspace --exclude heiwa-desktop --locked --no-default-features
bash scripts/check_ci_local.sh
```

Expected: `ALL GREEN — safe to push.` from the last command, except
`check_agent_baseline`, which fails on a dirty tree or a non-`dev` branch and
passes once the work is committed on `dev`.

The lesson recorded in the L3 ledger applies: run the command CI runs, not one
assumed to be equivalent. `cargo clippy` passing does not cover
`cargo fmt --all -- --check`, and a new crate absent from the root lockfile
builds locally and fails CI's `--locked`.

- [ ] **Step 5: Refresh the layer stamps and commit**

```bash
bash scripts/check_l0_acceptance.sh
bash scripts/check_l1_acceptance.sh
bash scripts/check_l2_acceptance.sh
git add scripts/ci_rust_test_group.sh docs/superpowers/ledgers
git commit -m "chore(work): register heiwa_work with CI and open the work fabric ledger"
```

The stamps bind to an exact clean HEAD, so run them after the final commit, not
before. The Stop hook blocks a session that claims a layer complete without a
fresh stamp.

---

## Definition of Done for A1-a

- `heiwa work create` opens a durable Work and `heiwa work list` replays it from
  the journal on a fresh root.
- A Work event without a `work_id` is refused by the writer, not stored.
- A thread whose rows already carry a consistent `work_id` adopts it; conflicting
  ids fail explicitly and never merge or mint a third.
- A delta from a different fold is refused even when its base revision matches.
- Work created before enrolment reports `replicable: false`.
- Events that cannot be folded are counted and surfaced, never hidden.
- `bash scripts/check_ci_local.sh` is green and the ledger rows are `done` with
  their verification command recorded.
