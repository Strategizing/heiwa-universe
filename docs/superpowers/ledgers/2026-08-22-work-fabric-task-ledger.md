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

## Release A1-b — Workspace Coordinator

Plan: `docs/superpowers/plans/2026-08-24-work-fabric-a1b-workspace-coordinator.md`

| # | Step | Status | Verification |
|---|---|---|---|
| 1 | Single git process boundary | done | `cargo test -p heiwa_workspace` |
| 2 | Repository snapshot | done | `cargo test -p heiwa_workspace` |
| 3 | Canonical roots and symlink refusal | done | `cargo test -p heiwa_workspace` |
| 4 | Isolated worktree lifecycle | done | `cargo test -p heiwa_workspace` |
| 5 | Writer lease on the evidence stream | done | `cargo test -p heiwa_workspace` |
| 6 | Refusal boundary for uncommitted work | done | `cargo test -p heiwa_workspace` |
| 7 | Bounded diff projection | done | `cargo test -p heiwa_workspace` |
| 8 | Test projection | done | `cargo test -p heiwa_workspace` |
| 9 | Workspace operator events | done | `cargo test -p heiwa_workspace -p heiwa-session` |
| 10 | Integration through journal and repository | done | `cargo test -p heiwa_workspace --test workspace_core` |
| 11 | `heiwa workspace` command | done | `cargo test -p heiwa-shell --bin heiwa cmd::workspace` |
| 12 | CI grouping and ledger | done | `bash scripts/ci_rust_test_group.sh --check` |

## A1 cohesion repair — 2026-08-24

Independent post-feature review found invariants that the component tests did
not compose across journal, Work, and Workspace boundaries.

| # | Repaired invariant | Status | Verification |
|---|---|---|---|
| 1 | One capability has one atomic lease winner across transports | done | `cargo test -p heiwa_evidence --test state` |
| 2 | Corrupt lease state fails closed and expired leases close before succession | done | `cargo test -p heiwa_evidence --test state` |
| 3 | Work, workspace leases, and workspace events share the resolved evidence root | done | `cargo test -p heiwa-shell --bin heiwa cmd::work` |
| 4 | Failed workspace preparation removes its clean worktree and revokes its lease | done | `cargo test -p heiwa-shell --bin heiwa cmd::workspace` |
| 5 | Workspace preparation requires durable Work and appends `workspace_prepared` | done | `cargo test -p heiwa-shell --bin heiwa cmd::workspace` |
| 6 | Work folding preserves global operator-cursor order across threads | done | `cargo test -p heiwa-shell --bin heiwa cmd::work` |
| 7 | Work IDs are safe path/ref components; client deltas cannot cross Work or regress revision | done | `cargo test -p heiwa_work -p heiwa_workspace` |

## Release A1-c — Work-bound execution and tri-surface delivery

Plan: `docs/superpowers/plans/2026-08-25-work-fabric-a1c1-work-bound-turns.md`

Release A1-c is **in progress**. A1-c1 closed the durable identity gap between
Work and the existing operator/Action Gate runtime. A1-c2
(`docs/superpowers/plans/2026-08-26-work-fabric-a1c2-worker-and-pane-identity.md`)
adds a provider-owned worker running inside the prepared worktree and a durable
pane bound to it. Neither claims tri-surface or restart-recovery completion.

| # | Step | Status | Verification |
|---|---|---|---|
| 1 | Work-scoped turn admission requires durable Work/thread membership | done | `cargo test -p heiwa-session --test operator_service work_scoped` |
| 2 | Retry identity binds prompt, route policy, and Work | done | `cargo test -p heiwa-session --locked` |
| 3 | Route, approval, tool, artifact, receipt, cancellation, and terminal events preserve Work scope | done | `cargo test -p heiwa-shell operator_work_scoped` |
| 4 | Bounded redacted Work-session projector over global cursor order | done | `cargo test -p heiwa_work --test work_session` |
| 5 | `heiwa work show <work-id>` renders the canonical session projector | done | `cargo test -p heiwa-shell cmd::work` |
| 6 | Provider-owned worker runs inside the prepared Work workspace | done | `cargo test -p heiwa-shell --bin heiwa cmd::worker` |
| 7 | Durable terminal pane binds to Work and worker identity | done | `cargo test -p heiwa_worker --test runs` |
| 8 | Home, Work, and Agent surfaces agree on Work/revision/cursor | pending | A1-c3 |
| 9 | Restart recovery exposes stale/closed worker and pane truth without repeating effects | pending | A1-c3 |
| 10 | Additive exact-HEAD `scripts/check_work_fabric_a1_acceptance.sh` | pending | A1-c3 |

### A1-c2 review repair — 2026-08-28

| # | Repaired invariant | Status | Verification |
|---|---|---|---|
| 1 | Prepared worker ownership and per-process run identity are distinct | done | `cargo test -p heiwa-shell --bin heiwa cmd::worker` |
| 2 | Repeated runs survive both the run fold and canonical Work-session projection | done | `cargo test -p heiwa-shell --bin heiwa cmd::worker` |
| 3 | `heiwa work run --json` emits one JSON document while retaining both child streams in bounded pane evidence | done | `cargo test -p heiwa-shell --test work_run` |
| 4 | Reader-thread panics reach the caller instead of becoming clean worker results | done | `cargo test -p heiwa-shell --bin heiwa cmd::worker` |

## Provider stream repair — 2026-09-06

Plane: Execution / Evidence. The Codex subscription adapter now consumes the
current CLI JSONL protocol. Completed assistant items and usage reach the
consumer; success requires both a completion event and a successful process
exit. Failure, malformed output, and cancellation no longer masquerade as
successful empty output or leave an idle adapter child running.

Verification: `cargo test -p heiwa-provider --locked --test codex_cli` passes
15 behavior cases plus the isolated child-process driver. Coverage includes
current and legacy output, usage, provider failure, invalid JSONL/UTF-8,
nonzero/missing completion, stderr pressure, literal prompt arguments, spawn
failure, shutdown ordering, and consumer cancellation. The target is included
in `scripts/ci_rust_test_group.sh`'s runtime integration lane.

This repairs the provider transport contract; A1-c3's surface agreement and
restart-recovery rows remain pending. It does not establish native Codex
continuation, tool-effect receipts, API migration, or live model entitlement.

### Dependency audit follow-up

The full local gate at `744ac65b` passed Rust, web, Python/product, baseline,
and L0-L2 acceptance checks, but failed `verify_security` on three pre-existing
Python dependency findings. The lockfiles now select MkDocs Material `9.7.7`
and Banks `2.4.5`, the patched versions for
[CVE-2026-73295](https://github.com/squidfunk/mkdocs-material/releases/tag/9.7.7)
and [CVE-2026-71492](https://github.com/masci/banks/security/advisories/GHSA-x8wg-4xgc-vr54).
`uv run --locked --extra docs python -m mkdocs build --strict` passes;
`uv run --locked --all-extras --python 3.14 python -m pytest -q` from
`runtime/python` passes all 13 sidecar tests.

At `8323492d`, the repeated security gate passed every check except the runtime Python audit:
`heiwa-sidecar[llama] -> llama-index-core -> nltk 3.10.3` retains
`PYSEC-2026-3740` / `CVE-2026-81726`.
[Upstream lists no patched version](https://github.com/nltk/nltk/security/advisories/GHSA-8mgp-746c-j5xp)
as of 2026-09-06; PyPI's latest release is still `3.10.3`. No advisory ignore or
audit exclusion was added. That revision remained blocked pending a verified
replacement/removal of that optional dependency path or a patched upstream
release.

### Optional dependency removal — 2026-09-06

Plane: Execution / Evidence. Repository inspection found no sidecar operation
using LlamaIndex APIs; `check_deps` only reports whether its module is importable.
The unused `llama` installation extra is now retired. Regenerating the lockfile
removes 42 packages, including LlamaIndex, NLTK, and Banks, with no additions or
version changes among retained packages. Existing environments can reconcile
with `uv sync --extra dev` from `runtime/python`.

The `llama_index` diagnostic result key is preserved. Characterization tests
cover both an externally installed module and its absence; all 14 sidecar tests
pass after syncing the reduced all-extras environment. A real JSONL subprocess
probe also passes health, dependency checks, echo, and shutdown with LlamaIndex
absent. Sidecar descriptions now distinguish import diagnostics from execution
capability.

`bash scripts/verify_security.sh` passes with zero failures and zero warnings;
both Python dependency audits report no known vulnerabilities. No advisory
ignore or audit exclusion was added. This resolves the dependency blocker;
live provider entitlement, remote CI, and installed-runtime promotion remain
outside these local proofs. A1-c3 remains pending.

## Development and publishing refactor — 2026-09-07

Plane: Execution / Evidence. The delivery harness now records per-check logs
and atomic local verification receipts with source identity and worktree state.
Required L0-L2 gates fail when absent. A1 remains explicitly deferred. Native
desktop checks are explicit in the full local profile and remain required in
PR CI. Sidecar tests/lint share a locked local/CI entry point; the existing
required aggregate rejects failed, cancelled, or skipped Python checks.

Active workflows use GitHub-hosted runners, checked across all workflow files
and static matrices. Releases validate the resolved tag's declarations with
trusted main validators and build from the resolved commit. Containers reuse
the verified release-byte packaging path. Agent instructions now share one
authorization and evidence contract.

Focused verification covers source changes, missing gates, process cleanup,
interruption, concurrent receipts, aggregate failure propagation, retired
runners, and tag/main version divergence. Exact committed local and remote
results belong in their generated receipts; these changes do not complete A1.

## Deferred with reason

- `work_node_bound` and `prior_history_digest` (WF-R15) need an enrolled mesh
  node. The attested-prefix design exists so binding adds a later event without
  changing an earlier one, so building the type now would produce something
  nothing can emit.
- `scripts/check_work_fabric_a1_acceptance.sh` lands in A1-c3, when the whole
  A1 checkpoint can pass. A1-c1 intentionally cannot satisfy the worker, pane,
  tri-surface, or restart-recovery rows.
- Multi-repository coordination (`WorkTaskGraphV1`, scope reservation,
  barriers, publication sagas) is Release A2. A1-b's lease is per repository.
- Interactive pane operations — `send`, `split`, `focus`, `pause`, `resume`
  from the Terminal Runtime contract need a PTY adapter. A1-c2 delivers
  `create`, `read`, and `stop` over pipes; the rest wait for that adapter
  rather than being faked through a pipe that cannot carry them.
- Restart reattach is row 9, in A1-c3. A worker whose process is gone folds as
  `stale` rather than being resurrected.
- A worker's parent, and the separate tool/filesystem/network/budget/action
  leases the spec's "Legitimate Workers" lists, are not on `WorkerIdentity`.
  A1-c2 has exactly one writer lease and no child workers, so those fields
  would have nothing that could populate them.
- `SandboxMode::Worktree` remains unwired. A1-c2 runs the worker in the
  prepared worktree with a cleared environment, but `heiwa_protocol`'s sandbox
  modes still have no backend behind them; naming one would claim an enforced
  control that is only declared.
- Commit and push from a worktree need the Action Gate, which is A1-c.
- Upstream divergence needs a remote and a fetch, which is a network effect
  belonging with the GitHub Collaboration Service in Release B.

## Next experimental slice

- A1-c3 — make Home, Work, and Agent agree on Work/revision/cursor; expose
  stale and closed worker and pane truth after a restart without repeating
  effects; and land `scripts/check_work_fabric_a1_acceptance.sh` so the whole
  A1 checkpoint can pass at an exact HEAD.
