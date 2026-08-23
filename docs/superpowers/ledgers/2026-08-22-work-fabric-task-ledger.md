# Work Fabric — Task Ledger

Contract: `docs/superpowers/specs/2026-08-22-heiwa-work-fabric-design.md`
Plan: `docs/superpowers/plans/2026-08-22-work-fabric-a1a-durable-work-core.md`
Started: 2026-08-22

Status is what is true at HEAD, not what is intended. A row moves to done only
when its verification runs.

## Release A1-a — Durable Work core

| # | Step | Status | Verification |
|---|---|---|---|
| 1 | `work_id` on the operator event | done | `cargo test -p heiwa_evidence` |
| 2 | `work_created` / `work_linked` types and scope validation | done | `cargo test -p heiwa-session` |
| 3 | `heiwa_work` crate and the Work aggregate | done | `cargo test -p heiwa_work` |
| 4 | Work event builders and readers | done | `cargo test -p heiwa_work` |
| 5 | Projector fold, damage counted | done | `cargo test -p heiwa_work` |
| 6 | Migration: adopt before generate | done | `cargo test -p heiwa_work` |
| 7 | Snapshot and epoch-guarded deltas | done | `cargo test -p heiwa_work` |
| 8 | Integration through the real journal | done | `cargo test -p heiwa_work --test work_core` |
| 9 | `heiwa work` command | done | `cargo test -p heiwa-shell --bin heiwa cmd::work` |
| 10 | CI grouping and ledger | done | `bash scripts/ci_rust_test_group.sh --check` |

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
