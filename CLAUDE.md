# CLAUDE.md — heiwa-universe

This repository builds Heiwa, a local-first AI runtime and enterprise platform. Claude Code is one wrapped provider surface inside Heiwa, not the product itself.

Naming:

- **Heiwa** = product/app/runtime/CLI/packages/docs.
- **Heiwa Limited** = company/publisher/legal identity.
- **Heiwa Universe** = this repo, `Heiwa-Limited/heiwa-universe`, public on GitHub since the v0.1.0 release. Treat everything committed here as published.

## Claude's Role Here

- Claude Code is a peer executor alongside Codex, Gemini CLI, Grok, Antigravity, and local model runtimes.
- Claude owns its own native tools, system prompts, auth semantics, model availability, and quota behavior.
- Heiwa adds repo-local context, routing, evidence, shell ergonomics, and cross-provider normalization.
- Do not write docs or code that implies Heiwa owns Claude's inference internals.

## Required Reading

Before touching runtime or architecture work, read in this order:

1. `HEIWA.md`
2. `AGENTS.md`
3. `.claude/settings.json`
4. `.claude/settings.local.json`

## Current Product Truth

- The installed `heiwa` runtime is the current product center.
- `apps/heiwa_shell/` is the primary operator surface in this repo.
- `apps/heiwa_core/` contains the Rust execution kernel and hosted runtime path.
- Evidence-plane work lives in `crates/heiwa_evidence/` (JSONL journal truth: envelopes, locking, replay, recovery, compaction); core and orchestrator consume it through their `evidence/` shims. Lance is wired behind the `lance` feature in `crates/heiwa_embed/`, selected via `embedding.backend`. STDB was extracted 2026-07-15.
- Legacy surfaces (old Hub, CLI, limbs) were removed from the tree on 2026-07-06; they live in git history and `~/heiwa_archive/`. Do not treat them as work targets.
- Web and `/code` surfaces are later work. Do not overstate them.

## Provider Truth

Heiwa wraps provider-owned runtimes:

- Claude Code
- Codex
- Gemini CLI
- Grok
- Antigravity
- Ollama and later local runtimes

Integration maturity is not identical across them. Be explicit about what is truly wired today.

## Shared Peer Truth

Use corrected peer framing before architecture or parity work:

- Hermes is Python, server/VPS-friendly, terminal-first. It proves learning loop,
  skills, FTS5 recall, Honcho user modeling, messaging gateway, cron delivery,
  MCP, provider switching, and terminal backends. Do not call it a worker mesh.
- OpenHuman is Rust + Tauri/CEF with local memory plus managed default services.
  It proves consumer desktop onboarding, Memory Tree, Obsidian vault,
  Composio/OAuth integrations, TokenJuice, and voice/meeting surface. Do not
  call it pure local-first.
- Heiwa's defensible difference: provider-peer MacBook owner seat, local runtime
  authority, approvals, receipts, local-first evidence (GitHub sync planned,
  redaction-gated), and provider-owned runtime truth.
- Biggest current gap: connector/tool breadth and compression/learning loop.
  Do not imply parity until code proves it.

## Active Build: Work Fabric A1 (autonomous)

Contract: `docs/superpowers/specs/2026-08-22-heiwa-work-fabric-design.md`. It is
the product-sequencing authority after L3 and supersedes the roadmap's post-L3
sequencing; it does not erase accepted layers.

Ledger (repo truth, update in the same commit as the work):
`docs/superpowers/ledgers/2026-08-22-work-fabric-task-ledger.md`. Status is what
is true at HEAD, not what is intended.

Acceptance: a release is complete only when its acceptance script passes at
HEAD and writes its stamp. `scripts/hooks/stop_ledger_gate.sh` enforces this
against every ledger and blocks a stop on an unverified completion claim. The
stamp is deliberately refused on a dirty tree.

Stamps are written at an exact HEAD but read with scope. Each acceptance script
declares an `# acceptance-scope:` line; an older stamp still counts when it is
an ancestor of HEAD and nothing under that scope changed between the two
commits. So a docs- or ops-only commit does not force a full re-run, while any
change under `apps/` or `crates/` does. A script with no declared scope falls
back to exact HEAD.

| Release | Ledger section | Acceptance | State |
| --- | --- | --- | --- |
| L0-L2 | `2026-08-14-L0-L1-task-ledger.md` | `scripts/check_l{0,1,2}_acceptance.sh` | accepted prerequisites |
| L3 | `2026-08-18-L3-calendar-mail-connectors` spec + its ledger | connector spec | Apple Calendar lane complete; Google blocked on account setup |
| Work Fabric A1 | `2026-08-22-work-fabric-task-ledger.md` | `scripts/check_work_fabric_a1_acceptance.sh` (lands in A1-c3) | **active** |

Escalate to Devon only for product-policy changes, irreversible/destructive
actions, or credentials.

## Branch Topology

```
experimental/* → protected dev → dev-to-main promotion PR → sync merge back to dev
```

`AGENTS.md` → "Promotion rule" is authoritative for the full invariants; do not
restate them here. What matters at the keyboard:

- Never commit on `main` or `dev` directly; branch first, as a separate step.
- Claude's experimental prefix is `experimental/*` (Codex uses `codex/*`).
- Verify with `HEIWA_BRANCH_MODE=experimental bash scripts/check_ci_local.sh`
  before opening the PR to `dev`.
- Another agent session may push to `dev` too — fetch before fixing, and take
  theirs when it landed first.
- `gh pr merge` is gated: drive a PR to CLEAN, then hand off to Devon.

## Commands

```bash
cargo build --workspace
cargo test -p heiwa-shell --test smoke -- --nocapture
cargo test -p heiwa-loop -- --nocapture
bash scripts/check_agent_baseline.sh
```

Use targeted crate tests before claiming runtime progress. Run the baseline gate
before closing repo-health, promotion, or peer-agent handoff work.

## Hard Rules

- local-first truth over web-first framing
- provider-owned semantics stay provider-owned
- Backend is Lance + GitHub: text truth in git, Lance derived recall index, SQLite hot state. No hosted authority plane.
- GitHub is the distribution surface; a cloud/VPS plane is deferred until traction warrants it
- honesty over completeness theater
