# Agent Baseline Workflow

Status: canonical development and publishing workflow for Heiwa.
Scope: provider agents working in this checkout.
Plane: Evidence — this workflow keeps repo truth inspectable before Execution slices and before any remote promotion.

## Non-negotiables

1. **Repo truth first.** Read `HEIWA.md`, `AGENTS.md`, and `docs/local-self-operation.md` before architecture, runtime, promotion, or remote work.
2. **Experimental branches feed `dev`; `dev` feeds `main`.** Agents create a short-lived branch from current `dev` in a temporary worktree under `.worktrees/` or `.claude/worktrees/`, commit there, and merge it into protected `dev` only through a reviewed, green pull request. `dev` is integration; `main` is production. Direct commits or pushes to either protected branch are forbidden.
3. **Authorization follows the outcome.** Apply the [shared operating contract](../AGENTS.md#operating-contract). An explicit publishing request covers normal commit, push, PR, review, and merge steps when the repo and destination are clear from context. Continue through authorized steps without asking for repeated command names or branch names. A local-only assignment stays local.
4. **Value-bearing execution.** Every work item must classify as Intake, Execution, Evidence, or out-of-scope. Size it by delivered value, not artificial smallness; split only where each part can ship independent value without leaving the product incomplete.
5. **Runtime split-brain is a blocker.** Port `7474` is installed product runtime. Checkout verification uses `7475` or another temporary port and the agent stops what it starts.
6. **Handoffs use the repo house style.** Include: `$caveman; repo truth first; ideate, build, execute, and ship real value only, regardless of slice size; verify; use reasonable workarounds; report only true blockers.`
7. **Do not mutate `vendor/` by accident.** Current `vendor/oss-lifts` material is ignored local quarantine/reference. Do not add, remove, depend on, or promote it without an explicit tracked-vendor assignment.

## Development loop

Inspect the owning code and contracts, implement the authorized change, run
checks that exercise changed behavior and meaningful failure boundaries, then
review the diff. Add regression coverage for durable bugs when feasible. Do not
manufacture tests for simple prose edits or rerun broad suites after unchanged
successful results without a new reason.

Before promotion, run the full local gate with the correct branch mode:

```bash
HEIWA_BRANCH_MODE=experimental bash scripts/check_ci_local.sh
```

Use `--full` for local Lance and native desktop certification; those native
checks remain required in remote CI/certification regardless of local profile.
The default receipt explicitly records native desktop certification as deferred.
Each run retains separate step
logs and a structured receipt binding results to the checkout revision and
reported worktree state. Required gates fail when missing or unsuccessful;
deferred acceptance is recorded explicitly. Work Fabric A1 remains deferred
until its acceptance gate exists and passes. A local receipt establishes only
the checks actually run; it does not prove remote CI, release availability, or
installed-runtime behavior.

If a real decision or tool restriction blocks progress, first prepare everything
already authorized: diff or payload, exact target, verification, and recovery
path. Explain the remaining decision or actual restriction concisely. Preserve
existing user authorization; skill procedures do not create new permission
requirements. This follows OpenAI's [current model guidance](https://developers.openai.com/api/docs/guides/latest-model)
on auditing conflicting instructions, completing authorized work, preparing
reviewable outcomes, and using meaningful targeted tests.

## Local baseline gate

Run this before claiming a clean repo-health or promotion handoff:

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
- cached `origin/dev` exists by default and local ahead/behind can be reported; topology also checks cached `origin/main`
- tracked tree is clean
- untracked files are absent except ignored or explicit `vendor/` quarantine entries
- exactly one linked worktree owns the configured integration branch, `dev` by default
- runtime baseline pins, Dockerfile baseline, release metadata, and product-surface classification pass

During active edits, agents may smoke-test the gate logic without claiming a clean baseline:

```bash
HEIWA_BRANCH_MODE=experimental bash scripts/check_agent_baseline.sh --allow-dirty
```

`--allow-dirty` is development evidence only. An uncommitted handoff should name
the dirty files and the checks that passed; it must not claim a clean baseline
or promotion readiness. Do not commit, stash, reset, or discard existing work
merely to satisfy this gate. A clean promotion handoff runs without this flag.

## Remote pre-flight for authorized publishing

Resolve the repository, source branch, destination, and intended publication
from the request and current context. An instruction such as “publish this
change” authorizes the protected branch flow when that target is unambiguous;
it does not require a second instruction naming each command. Release tags and
deployment destinations must follow the requested outcome and release metadata.
Ask only if a material target or action remains ambiguous. Publishing does not
authorize unrelated messages, destructive cleanup, credential changes, or an
installed-runtime restart.

Read-only remote freshness and review checks are part of an authorized
publishing or remote-audit task. Capture the local context before mutating the
remote target:

```bash
git status --short --branch --untracked-files=all
git rev-parse HEAD
git remote -v
git worktree list --porcelain
```

Then run the remote-health checks appropriate to the operation:

```bash
# Authenticate without printing tokens. Stop dependent work if auth is blocked.
gh auth status
git ls-remote --heads origin dev main
git fetch origin dev main

# Confirm refs after fetch.
git rev-parse HEAD origin/dev origin/main
git rev-list --left-right --count origin/main...origin/dev
```

Before every merge, re-read the PR's current head and base, required check
results, mergeability, and unresolved review threads, including GraphQL review
threads when the CLI summary omits them. Confirm check and review evidence
belongs to the current head; an older green commit or resolved older review
does not establish readiness. Reproduce actionable findings against current
code, fix them, and rerun affected checks. Pass the verified head to the merge
command where supported so a concurrent push cannot change the reviewed merge.
Do not bypass branch protection or treat “mergeable” as sufficient review.

For a public release, verify CI and certification on the exact source commit
and prove the release/tag/checksum/source-branch authority:

```bash
bash scripts/check_release_metadata.sh
# Inspect CI and certification runs using their exact head SHA before release.
```

Use GitHub Releases as the binary authority. Active workflows use GitHub-hosted
runners; a custom-runner experiment must be isolated in a dispatch-only canary.
Verify published release assets and checksums after publication. Follow the
[installed-runtime contract](local-self-operation.md#install-and-update-authority)
only when installation is separately authorized, and record the installed
version/path and runtime behavior after an update.

Report completed stages with their receipts and remaining blockers. Local
checks, exact-head PR CI/review, source-commit release certification, published
assets, and installed behavior establish distinct facts.

## Vendor quarantine policy

Decision for the current checkpoint: root `vendor/` is ignored local quarantine. `vendor/oss-lifts` can stay on this machine as research lift material, but it is not part of the production remote checkpoint and should not appear in normal `git status`.

Tracked vendor code remains possible later, but only as an explicit slice:

- use `git add -f vendor/...` only after Devon assigns the tracked-vendor slice
- add/verify license and provenance notes
- classify the tracked path under `PRODUCT_SURFACE.md` as `vendored`
- add tests or docs proving why product code depends on the vendored material
- do not cite ignored `vendor/` as product maturity evidence

## Agent-to-agent handoff shape

For substantial peer handoffs, include the following information; omit empty
sections and combine related gaps. Short user-facing reports can be concise.

```text
$caveman; repo truth first; ideate, build, execute, and ship real value only, regardless of slice size; verify; use reasonable workarounds; report only true blockers.

Acquired data:
- ...

Authorized outcome and remaining scope:
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

Do not hide dirty files, failed commands, stale refs, or unverified remote state.
Preserve authorization and make the next action executable without restarting
the approval or discovery process.
