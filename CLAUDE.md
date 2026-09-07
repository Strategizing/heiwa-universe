# CLAUDE.md — heiwa-universe

Claude Code is a provider-owned peer executor inside Heiwa. It owns its native
tools, system prompts, authentication, sessions, model inventory, and quotas.
Heiwa owns the local runtime, routing, and evidence around that provider surface.

## Shared Operating Contract

Use [`AGENTS.md`](AGENTS.md#operating-contract) for authorization, implementation,
review, testing, and reporting. Explicit user authorization persists across
turns; provider skills and local guidance must not add a second approval round
to already authorized work. This file supplies Claude-specific context only.

Before runtime or architecture changes, read:

1. [`HEIWA.md`](HEIWA.md) for product and architecture truth.
2. [`AGENTS.md`](AGENTS.md) for shared working rules.
3. [`docs/local-self-operation.md`](docs/local-self-operation.md) for runtime boundaries.

When diagnosing Claude configuration or hooks, inspect `.claude/settings.json`
and `.claude/settings.local.json` if present. Local settings and installed
plugins may differ by machine; do not edit provider-owned settings as a side
effect of repository work.

## Branches and Publishing

Claude uses `experimental/*`; Codex uses `codex/*`. Start from current `dev` and
follow the promotion rule in `AGENTS.md`: experimental PR -> protected `dev` ->
production PR to `main` -> synchronize `dev`. Never commit or push directly to
`dev` or `main`, bypass protection, or overwrite another agent's changes.

An authorized publishing task includes driving review and CI through merge.
Recheck the exact PR head, required checks, merge state, and unresolved review
threads immediately before merging. Use
[`docs/agent-baseline-workflow.md`](docs/agent-baseline-workflow.md) for the
pre-flight and receipts. A tool approval restriction still applies: prepare
the reviewable result and report the actual restriction if it blocks progress.

## Verification and Ledgers

Use targeted tests while iterating. Before promotion, run:

```bash
HEIWA_BRANCH_MODE=experimental bash scripts/check_agent_baseline.sh
HEIWA_BRANCH_MODE=experimental bash scripts/check_ci_local.sh
```

Use the appropriate branch mode on integration or post-promotion checkouts.
An uncommitted handoff may use the baseline's `--allow-dirty` development mode,
but must report the dirty tree and cannot claim clean promotion readiness.

For Work Fabric work, consult the current design and ledger:

- `docs/superpowers/specs/2026-08-22-heiwa-work-fabric-design.md`
- `docs/superpowers/ledgers/2026-08-22-work-fabric-task-ledger.md`

Update a relevant ledger alongside the work it describes. A completion claim
requires its acceptance evidence. `scripts/hooks/stop_ledger_gate.sh` checks
ledger claims against acceptance stamps; a stamp is written only on a clean
tree. Older stamps are reusable only when the checker confirms an ancestor
revision and unchanged declared acceptance scope. That local scope check does
not replace exact-source-commit CI and certification for a public release.

The A1 acceptance gate remains deferred until implemented and passed. A plan,
partial feature, or passing prerequisite cannot establish A1 completion.
