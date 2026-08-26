# Work Fabric A1-c1 — Work-Bound Operator Turns Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make one operator turn durably belong to one existing `Work`, carry that identity through every route, approval, tool, artifact, receipt, cancellation, and terminal event, and expose the resulting state through one canonical Work-session snapshot.

**Architecture:** `StartTurnRequest` receives an optional `work_id`. `OperatorSessionService` validates that scoped submissions name a durable Work linked to the target thread and binds the idempotency key to that Work. `OperatorTurnRunner` carries the admitted scope into every runtime event rather than reconstructing identity later. `heiwa_work` folds the same append-ordered operator stream into `WorkSessionSnapshotV1`; the CLI renders that projector, so future Home, Work, and Agent surfaces can consume one contract instead of joining independent stores.

**Tech Stack:** Rust 2021, `heiwa_evidence` operator JSONL, `heiwa_session` sole-writer service, `heiwa_work` replayable projections, `heiwa_shell` CLI, `serde_json`, existing SHA-256 and UUID dependencies. No new external crates or storage engines.

---

## Scope and release boundary

This is the first cohesive delivery inside A1-c. It does **not** claim Release
A1 complete.

| Slice | Delivers | Status at plan creation |
|---|---|---|
| A1-c1 (this plan) | durable Work-bound turns; full runtime-event propagation; canonical Work-session projection; `heiwa work show` | implemented on experimental branch; delivery gates pending |
| A1-c2 | provider-owned worker launch and terminal-pane binding inside the prepared worktree | deferred to a fresh experimental branch |
| A1-c3 | Home/Work/Agent rendering agreement, restart UX, and `scripts/check_work_fabric_a1_acceptance.sh` | deferred until A1-c2 exists |

**Independently useful because:** after this slice, an execution, approval, tool
result, artifact, cancellation, and receipt can be queried as facts of one
durable Work. That closes the current identity hole even before a desktop pane
is launched.

## Design decisions

1. **Work membership is admitted before the first turn row.** A scoped request
   must name a `work_created`/`work_linked` relationship already present in the
   operator journal. Failure writes nothing.
2. **Idempotency includes `work_id`.** Reusing a client request id against a
   different Work is a conflict, even when prompt and route policy match.
3. **The envelope is authoritative.** Every event emitted by a scoped turn has
   `OperatorEvent.work_id`; payload parsing is never required to determine Work
   ownership.
4. **Unscoped behavior remains compatible.** Existing callers and historical
   JSON omit `work_id` and continue to behave exactly as before.
5. **The Work projector is read-only.** It folds append-ordered events and
   produces bounded collection rows; it introduces no new database, cache, or
   writer.
6. **This patch does not fake worker or pane state.** Those facts land only when
   A1-c2 can emit them from a real provider-owned process and terminal adapter.

## Existing substrate

- `crates/heiwa_evidence/src/operator.rs` already provides optional `work_id`
  on every event and a cursor-ordered paged journal.
- `crates/heiwa_session/src/operator.rs` owns turn admission, idempotency,
  validation, replay, cancellation, and the sole-writer transaction.
- `apps/heiwa_shell/src/operator.rs` owns ordered route, approval, tool,
  artifact, receipt, and terminal emission.
- `crates/heiwa_work/src/projector.rs` folds durable Work identity and revision.
- `crates/heiwa_work/src/snapshot.rs` already defines the epoch/revision/cursor
  delivery contract but has no concrete builder.
- `apps/heiwa_shell/src/cmd/work.rs` already reads the global journal in append
  order and is the narrow CLI surface for Work.

---

## Task 1: Bind turn admission and idempotency to durable Work

**Files:**

- Modify: `crates/heiwa_session/src/operator.rs`
- Modify: `crates/heiwa_session/tests/operator_service.rs`

- [ ] **Step 1: Write failing admission tests**

Add tests proving:

```rust
let mut request = StartTurnRequest::auto("request-1", "ship it");
request.work_id = Some("work-abc".to_string());
```

- is refused before any row when `work-abc` is unknown;
- is refused when the Work exists but the target thread is not linked;
- succeeds after `work_created` or `work_linked` establishes membership;
- writes `work_id` on `turn_started` and `user_message`;
- conflicts when the same client request id is retried with a different Work;
- remains source/JSON compatible when `work_id` is absent.

Run:

```bash
cargo test -p heiwa-session --test operator_service work_scoped -- --nocapture
```

Expected: FAIL because the request and materialized membership do not exist.

- [ ] **Step 2: Add the optional request/view identity**

Add `#[serde(default, skip_serializing_if = "Option::is_none")] pub work_id:
Option<String>` to `StartTurnRequest`, initialize it to `None` in `auto`, and
expose `work_id` on `OperatorTurnView`/the internal folded turn.

- [ ] **Step 3: Materialize durable Work/thread membership**

Extend `MaterializedJournal` with a derived map from Work id to linked thread
ids. Update it only from accepted current-schema `WorkCreated` and `WorkLinked`
events. Reset/rebuild behavior remains inherited from `sync_materialized`.

Do not import `heiwa_work` into `heiwa_session`: the dependency direction is
foundation → runtime. Validate the envelope and event relationship already
present in this crate rather than duplicating the Work aggregate.

- [ ] **Step 4: Admit and append atomically**

Inside the existing sole-writer transaction, reject a scoped request unless
its Work owns the target thread. Pass the Work id to `new_event` for
`turn_started`, `user_message`, and crash-recovered user-message rows. Extend
retry binding to compare the stored and requested Work ids.

- [ ] **Step 5: Run the focused and complete session tests**

```bash
cargo test -p heiwa-session --test operator_service work_scoped -- --nocapture
cargo test -p heiwa-session --locked
```

Expected: PASS.

## Task 2: Propagate admitted Work through the real operator runner

**Files:**

- Modify: `apps/heiwa_shell/src/operator.rs`
- Modify: `apps/heiwa_shell/src/operator_api.rs` or the actual request DTO file found by `rg`
- Modify: `apps/heiwa_shell/src/model_calls.rs` only if its public request already carries Work
- Test: `apps/heiwa_shell/src/operator.rs`
- Test: `apps/heiwa_shell/tests/operator_api.rs`

- [ ] **Step 1: Write a failing end-to-end runner test**

Create a durable Work/thread pair, submit a deterministic or model turn with
`request.work_id`, drain it, and assert every event from `turn_started` through
`turn_completed` carries the same Work id. Add a gated tool case asserting
`approval_requested`, `approval_decided`, `tool_call_completed`, and
`receipt_linked` are scoped too.

Run:

```bash
cargo test -p heiwa-shell --bin heiwa work_scoped -- --nocapture
```

Expected: FAIL because `runtime_event` currently sets `work_id: None`.

- [ ] **Step 2: Introduce one immutable runtime scope**

Carry `{ thread_id, turn_id, work_id }` from `TurnSubmission` into the spawned
runner. Keep the Work id with active-turn cancellation state. Make the runtime
event constructor accept that scope and copy its Work id into the envelope.

- [ ] **Step 3: Replace every runtime emission site**

Route planned/attempted/completed/failed, assistant lifecycle, approval
request/decision, tool lifecycle, artifacts, tests, receipts, blockers,
cancellation, and terminal events must all use the same admitted scope. Any
event emitted outside a turn remains unchanged.

- [ ] **Step 4: Preserve HTTP/API compatibility**

If the operator submission DTO is `StartTurnRequest` directly, serde defaults
are sufficient. If it maps fields explicitly, add an optional `work_id` field
and pass it through. Add a request-contract test for both omission and presence.

- [ ] **Step 5: Run shell operator gates**

```bash
cargo test -p heiwa-shell --bin heiwa work_scoped -- --nocapture
cargo test -p heiwa-shell --test operator_api --locked
cargo test -p heiwa-shell --bin heiwa --locked
```

Expected: PASS.

## Task 3: Build the canonical Work-session projection

**Files:**

- Add: `crates/heiwa_work/src/session.rs`
- Modify: `crates/heiwa_work/src/lib.rs`
- Modify: `crates/heiwa_work/src/snapshot.rs` only if a bounded diagnostic field is needed
- Add: `crates/heiwa_work/tests/work_session.rs`

- [ ] **Step 1: Write failing projector tests**

Build one ordered event stream containing two Works, one workspace, one scoped
turn, an approval, a tool completion, an artifact, and a receipt. Assert that
`build_work_session(events, work_id, epoch_seed)`:

- rejects an unknown Work;
- includes only rows whose envelope names the requested Work;
- exposes collections `work`, `threads`, `workspace`, `approvals`, `actions`,
  `artifacts`, `receipts`, and `blockers`;
- keys rows by stable event/domain ids;
- uses the Work aggregate revision separately from projection revision;
- carries the final supplied operator cursor;
- bounds each collection and reports truncation rather than silently growing.

Run:

```bash
cargo test -p heiwa_work --test work_session -- --nocapture
```

Expected: FAIL because no concrete builder exists.

- [ ] **Step 2: Implement a pure append-order fold**

Define `WorkSessionBuild` input containing ordered `CursorEvent` rows, an epoch
seed, and a per-collection limit. First fold Work identity through the existing
projector. Then select only events with the exact `work_id` and upsert bounded
summary rows. Never embed full artifact bodies, full diffs, prompts, tool
arguments, or secrets in the snapshot.

- [ ] **Step 3: Define stable collection schemas**

At minimum:

```text
work/<work_id>          aggregate identity, intent, status, revision
threads/<thread_id>     latest turn/status and cursor-visible timestamps
workspace/<repo_root>   worktree, branch, base commit, released state
approvals/<event_id>    call/request ids, risk, outcome, timestamp
actions/<call_id>       tool name, status, receipt reference, timestamp
artifacts/<artifact_id> bounded metadata and evidence references
receipts/<event_id>     kind and receipt reference
blockers/<event_id>     bounded reason/code only
```

Use JSON values at the existing snapshot boundary, but centralize every
projection shape in `session.rs`; consumers must not reinterpret raw events.

- [ ] **Step 4: Run Work tests**

```bash
cargo test -p heiwa_work --locked
```

Expected: PASS.

## Task 4: Surface the same projector through `heiwa work show`

**Files:**

- Modify: `apps/heiwa_shell/src/cmd/work.rs`
- Test: `apps/heiwa_shell/src/cmd/work.rs`

- [ ] **Step 1: Write a failing command test**

Create a Work and scoped turn, then assert the command helper returns a
`WorkSessionSnapshotV1` with the same Work id, revision, final operator cursor,
and turn/action/receipt rows.

- [ ] **Step 2: Reuse one append-order journal read**

Refactor the existing paged reader to retain `CursorEvent` rows. Use it for
both the list projection and `build_work_session`; do not scan each thread or
open a second store.

Add:

```text
heiwa work show <work-id> [--json]
```

Human output prints bounded summaries. JSON prints the versioned snapshot
contract future native surfaces consume.

- [ ] **Step 3: Run command tests**

```bash
cargo test -p heiwa-shell --bin heiwa cmd::work -- --nocapture
```

Expected: PASS.

## Task 5: Ledger, review, and experimental delivery

**Files:**

- Modify: `docs/superpowers/ledgers/2026-08-22-work-fabric-task-ledger.md`
- Modify: `scripts/ci_rust_test_group.sh` only if a new integration target is added

- [ ] **Step 1: Record A1-c1 truth without claiming A1 complete**

Add an A1-c section whose rows move to `done` only after the exact verification
runs. Keep worker/pane, tri-surface UI, restart UX, and the A1 acceptance gate
explicitly pending.

- [ ] **Step 2: Post-feature review**

Inspect:

```bash
git diff --check
git diff --stat origin/dev...HEAD
git diff origin/dev...HEAD -- crates/heiwa_session apps/heiwa_shell crates/heiwa_work
rg -n "work_id: None" apps/heiwa_shell/src/operator.rs
```

Confirm no scoped runner emission loses Work identity, no second writer/store
was added, sensitive bodies stay out of the Work snapshot, and all public
optional fields are backward compatible.

- [ ] **Step 3: Run focused and repository gates**

```bash
cargo test -p heiwa-session -p heiwa_work -p heiwa-shell --locked
bash scripts/ci_rust_test_group.sh --check
HEIWA_BRANCH_MODE=experimental bash scripts/check_agent_baseline.sh
HEIWA_BRANCH_MODE=experimental bash scripts/check_ci_local.sh
```

Expected: all green on the exact experimental HEAD.

- [ ] **Step 4: Commit, push, and open an experimental → dev PR**

Use conventional commits on `codex/experimental-work-fabric-a1c`. Push only
after the local pre-push gate passes. Open the PR against protected `dev`, wait
for every required check and review thread, fix findings on the experimental
branch, then merge only when green.

- [ ] **Step 5: Re-prove branch topology**

After merge:

```bash
git fetch origin --prune
git rev-list --left-right --count origin/main...origin/dev
```

Required outcome: `dev` is zero behind and strictly ahead of `main`. Remove the
merged experimental worktree/branch, leave the installed `7474` runtime
untouched, and report exact SHAs/checks/PR state.

---

## Self-review checklist

- [ ] Every scoped append uses envelope `work_id`; no payload-only ownership.
- [ ] Unknown/cross-thread Work submission writes zero rows.
- [ ] Retry binding includes prompt, route policy, and Work.
- [ ] Historical unscoped request/event JSON remains readable.
- [ ] Snapshot reads global cursor order and uses bounded pages/collections.
- [ ] Full prompts, tool arguments, artifact bodies, diffs, and auth material are absent from the snapshot.
- [ ] No worker/pane/A1-complete claim appears in code, docs, or PR text.
- [ ] Experimental branch is based on current `dev`; final merge targets `dev` only.
