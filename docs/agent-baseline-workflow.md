# Agent Baseline Workflow

Status: canonical local-agent workflow for Heiwa repo health.
Scope: Claude Code, Codex, Gemini CLI, Antigravity, Hermes, and any future Class 3 executor working in this checkout.
Plane: Evidence — this workflow keeps repo truth inspectable before Execution slices and before any remote promotion.

## Non-negotiables

1. **Repo truth first.** Read `HEIWA.md`, `AGENTS.md`, and `docs/local-self-operation.md` before architecture, runtime, promotion, or remote work.
2. **Experimental branches feed `dev`; `dev` feeds `main`.** Agents create a short-lived branch from current `dev` in a temporary worktree under `.worktrees/` or `.claude/worktrees/`, commit there, and merge it into protected `dev` only through a reviewed, green pull request. `dev` is integration; `main` is production. Direct commits or pushes to either protected branch are forbidden.
3. **No remote operations by drift.** `git fetch`, `git pull`, `git push`, `gh run`, `gh release`, `spacetime publish`, `wrangler deploy`, and equivalent network-promotion commands require an explicit assignment for that remote operation.
4. **Value-bearing execution.** Every work item must classify as Intake, Execution, Evidence, or out-of-scope. Size it by delivered value, not artificial smallness; split only where each part can ship independent value without leaving the product incomplete.
5. **Runtime split-brain is a blocker.** Port `7474` is installed product runtime. Checkout verification uses `7475` or another temporary port and the agent stops what it starts.
6. **Handoffs use the repo house style.** Include: `$caveman; repo truth first; ideate, build, execute, and ship real value only, regardless of slice size; verify; use reasonable workarounds; report only true blockers.`
7. **Do not mutate `vendor/` by accident.** Current `vendor/oss-lifts` material is ignored local quarantine/reference. Do not add, remove, depend on, or promote it without an explicit tracked-vendor assignment.

## Local baseline gate

Run this before closing a repo-health slice, before local promotion, and before handing work to another agent:

```bash
bash scripts/check_agent_baseline.sh
```

On an experimental branch, declare the topology explicitly so the same full
local gate proves the branch descends from current `dev`:

```bash
HEIWA_BRANCH_MODE=experimental bash scripts/check_agent_baseline.sh
```

`dev` must never be behind cached `origin/main`. At integration and promotion
handoffs it must also be ahead by at least one value-bearing commit. Immediately
after a `dev` -> `main` promotion, synchronize the merge commit and use the
bounded transition mode until the next experimental PR restores the lead:

```bash
HEIWA_BRANCH_MODE=post-promotion bash scripts/check_agent_baseline.sh
```

Do not add empty or sentinel commits merely to make `dev` appear ahead.

The gate is intentionally local-only. It does not fetch, push, call GitHub, or verify remote CI. It checks:

- branch topology matches the declared mode (`dev` ahead for integration, a branch descended from `dev` for experimental work, or synchronized `dev` during post-promotion handoff)
- cached `origin/main` ref exists and local ahead/behind can be reported
- tracked tree is clean
- untracked files are absent except ignored or explicit `vendor/` quarantine entries
- no duplicate linked worktree owns `main`
- runtime baseline pins, Dockerfile baseline, release metadata, and product-surface classification pass

During active edits, agents may smoke-test the gate logic without claiming a clean baseline:

```bash
HEIWA_BRANCH_MODE=experimental bash scripts/check_agent_baseline.sh --allow-dirty
```

`--allow-dirty` is for development only. A final baseline handoff must run without it.

## Remote pre-flight gate — explicit assignment only

Remote health is a separate operation. Before any push, release, remote CI reliance, SpacetimeDB publish, or Cloudflare promotion, the assigning message must name the remote operation and target branch.

When assigned, capture and report these artifacts before mutating the remote target:

```bash
git status --short --branch --untracked-files=all
git rev-parse --short=8 HEAD
git rev-parse --short=8 origin/main
git rev-list --left-right --count origin/main...main
git worktree list --porcelain
```

Then run the remote-health checks appropriate to the operation:

```bash
# Network/auth freshness. Do not proceed if any command prompts unexpectedly.
git ls-remote --heads origin main
git fetch --prune --tags origin main

# Confirm refs after fetch.
git rev-parse --short=8 HEAD
git rev-parse --short=8 origin/main
git rev-list --left-right --count origin/main...main

# GitHub CI evidence for the exact target.
gh auth status
gh run list --branch main --limit 10
gh run view <run-id> --log-failed
```

For release/promotion work, also prove the exact source authority:

```bash
bash scripts/check_release_metadata.sh
# Then document the release/tag/checksum/source branch that will become authority.
```

Do not substitute local green checks for remote health. Local checks answer “is this checkout healthy?” Remote checks answer “is the public target fresh, authenticated, and verified?”

## Vendor quarantine policy

Decision for the current checkpoint: root `vendor/` is ignored local quarantine. `vendor/oss-lifts` can stay on this machine as research lift material, but it is not part of the production remote checkpoint and should not appear in normal `git status`.

Tracked vendor code remains possible later, but only as an explicit slice:

- use `git add -f vendor/...` only after Devon assigns the tracked-vendor slice
- add/verify license and provenance notes
- classify the tracked path under `PRODUCT_SURFACE.md` as `vendored`
- add tests or docs proving why product code depends on the vendored material
- do not cite ignored `vendor/` as product maturity evidence

## Agent-to-agent handoff shape

Use this shape for peer handoffs and final repo-health reports:

```text
$caveman; repo truth first; ideate, build, execute, and ship real value only, regardless of slice size; verify; use reasonable workarounds; report only true blockers.

Acquired data:
- ...

Missing data:
- ...

Needed data:
- ...

Changed files:
- ...

Verification evidence:
- command -> result

Next executable slice:
- ...
```

Do not hide dirty files, failed commands, stale refs, or unverified remote state. The team stays symbiotic by making the next agent’s first step obvious and safe.
