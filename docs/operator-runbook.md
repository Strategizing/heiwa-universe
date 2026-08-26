# Operator Runbook

## Boot sequence

Read these before runtime, architecture, promotion, or remote work:

1. `HEIWA.md`
2. `AGENTS.md`
3. `docs/local-self-operation.md`
4. `docs/agent-baseline-workflow.md`

`HEIWA.md` wins when older docs disagree.

## Local agent baseline

Use the local-only gate before closing a repo-health slice, before local runtime promotion, or before handing work to another agent:

```bash
bash scripts/check_agent_baseline.sh
```

During active edits only, agents may test the gate shape with:

```bash
HEIWA_BRANCH_MODE=experimental bash scripts/check_agent_baseline.sh --allow-dirty
```

A final handoff must run without `--allow-dirty`.

Use experimental branches for every change. The delivery sequence is
experimental -> protected `dev` -> protected `main`; direct pushes to either
long-lived branch are forbidden.

## Basic local checks

Pick the narrowest check that covers the changed surface, then broaden only after it passes.

Repo-health baseline:

```bash
bash scripts/check_runtime_baseline.sh
bash scripts/check_heiwa_core_dockerfile.sh
bash scripts/check_release_metadata.sh
bash scripts/audit_product_surface.sh
bash scripts/check_agent_baseline.sh
```

Rust/runtime examples:

```bash
cargo test --offline -p heiwa-protocol -p heiwa_mcp -p heiwa_evidence -p heiwa-shell
cargo test --offline -p heiwa-core --test drex_provider_routing --test drex_scoring --test run_receipts --test worker_mesh
cargo test --offline -p heiwa-shell --test agentic_smoke
```

Docs/connectors/audit examples:

```bash
python3 scripts/validate_connector_manifests.py
bats tests/audit/test_connector_manifests.bats
bats tests/audit/test_audit_product_surface.bats
uv run --extra docs mkdocs build --strict
```

## Remote pre-flight

Remote operations require explicit assignment. Do not drift from local repo health into network promotion.

Remote operations include `git fetch`, `git pull`, `git push`, `gh run`, `gh release`, `spacetime publish`, `wrangler deploy`, and equivalent publish/sync commands.

When assigned, capture baseline first:

```bash
git status --short --branch --untracked-files=all
git rev-parse --short=8 HEAD
git rev-parse --short=8 origin/main
git rev-list --left-right --count origin/main...main
git worktree list --porcelain
```

Then verify network auth, fresh refs, and CI/release evidence for the exact remote target before mutation. See `docs/agent-baseline-workflow.md#remote-pre-flight-gate--explicit-assignment-only`.

## Vendor quarantine

`vendor/oss-lifts` is ignored local research quarantine only. Until a tracked-vendor assignment exists, do not add, delete, import from, or cite it as product evidence.

## Public surface rule

If a surface is not verified by tests or build checks, it should not be described as stack-complete in docs, README, or the static web shell.
