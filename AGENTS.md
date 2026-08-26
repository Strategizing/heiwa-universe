# AGENTS.md — heiwa-universe

This repository builds the Heiwa full stack. The current product center of gravity is the installed `heiwa` runtime on the user's machine.

## Canonical Product Truth

- **Heiwa** is the product identity: app, runtime, CLI, packages, and docs.
- **Heiwa Limited** is the company/publisher/legal identity.
- **Heiwa Universe** is this repository: `Heiwa-Limited/heiwa-universe`, public on GitHub since the v0.1.0 release. Treat everything committed here as published; secret scanning and the security gates are the only thing between a commit and the world.
- **`heiwa`** is the primary installed operator surface.
- **DREX** is the internal execution kernel.
- **Per-user local state** under the resolved config root (`~/.heiwa` by default) is the runtime truth on each machine. `crates/heiwa_config::HeiwaPaths` is the sole resolver; the product assumes N users, not one seat.
- **Lance + GitHub** are the backend (pivot 2026-07-15): text truth (JSONL/markdown) local-first — GitHub sync planned, redaction-gated — with Lance as the derived local recall index. SpacetimeDB is retired; code extracted from the tree. Journal service: `crates/heiwa_evidence/`.
- **GitHub** is distribution and CI. Evidence sync is planned and redaction-gated, not live. **Cloudflare** is DNS utility only.

Compression:

> Rust proposes and executes, local text truth records, Lance recalls, `heiwa` presents; future redacted projections may sync through GitHub.

## Current Repo Spine

| Path                       | Role                                                                                |
| -------------------------- | ----------------------------------------------------------------------------------- |
| `apps/heiwa_shell/`        | Installed `heiwa` runtime and shell surface                                         |
| `apps/heiwa_app/`          | Companion visual shell for the same runtime; web client today, native wrapper later |
| `apps/heiwa_core/`         | Rust execution kernel and hosted runtime path                                       |
| `apps/heiwa_orchestrator/` | DREX orchestration, scoring, and local evidence runtime work                        |
| `crates/heiwa_provider/`   | Provider normalization and adapter surfaces                                         |
| `crates/heiwa_install/`    | Install and doctor flows                                                            |
| `crates/heiwa_session/`    | Local session daemon primitives                                                     |
| `crates/heiwa_repl/`       | REPL parsing and footer telemetry                                                   |
| `crates/heiwa_loop/`       | Bounded loop workflow                                                               |
| `packages/heiwa_sdk/`      | Python compatibility and migration surface                                          |

## Architecture Direction (April 2026)

- User-functionality stack is **Rust + TypeScript + Shell**, developed against a local machine first (currently the maintainer's MacBook) but written for any user's machine.
- **Rust** owns the authoritative state layer, orchestration, routing, and future DREX execution logic.
- **TypeScript** owns companion visual surfaces and typed client contracts.
- **Shell** remains the bootstrap and operator glue layer for the local runtime plus future Linux/WSL execution.
- The Python Hub and cognition packages are still live in the repo, but they are prototype and compatibility surfaces, not the long-term control plane.

## Provider Truth

Heiwa wraps provider-owned runtimes. It does not own their internals.

- Claude Code, Codex, Gemini CLI, Antigravity, and Grok remain provider-owned CLI surfaces.
- Direct-API adapters ship alongside the CLI adapters (Anthropic, OpenAI,
  Google). Adapter selection lives in `heiwa_provider::routing`, never in a
  surface binary; adapters do transport and `StreamEvent` translation only,
  with routing, quota, and cost staying with DREX and `heiwa_quota`.
- Providers own their own system prompts, auth semantics, session behavior, cloud model inventory, and native quotas.
- Ollama and other local runtimes remain local-model providers, not Heiwa-native models.
- Heiwa adds local install/auth UX, routing, evidence, bounded loops, and operator coherence across those surfaces.

Be honest about maturity:

- discovery and wrapping may exist before parity does
- a provider may be known before it is fully loop-capable
- hosted surfaces exist in the repo, but they are not the current product center
- `apps/heiwa_app` is the companion visual shell path today, not a fully native desktop runtime yet

## Operator and Infra Truth

- The runtime root is per-user and resolved by ConfigRoot; on the maintainer's machine that is `~/.heiwa/`. Product code must never assume that path or that operator.
- This checkout plus the local runtime root are the current source-of-truth/server for user functionality on a development machine.
- Users/operators should not have to think about the evidence backend directly.
- GitHub is source, CI, and release distribution.
- Cloudflare is paused public edge and later remote-surface infrastructure.

## Agentic Runtime Workflow

Use [`docs/local-self-operation.md`](docs/local-self-operation.md#agentic-runtime-workflow) before starting, stopping, probing, or changing the local app runtime.

- Treat `7474` as the installed product runtime; after code edits, verify the current checkout on a temporary alternate port such as `7475`.
- Never assume a reachable localhost app is the binary you just changed; check `cli_path`, port, and endpoint behavior.
- If a new API endpoint returns `index.html`, assume stale or wrong runtime until proven otherwise.
- Prompt for update/restart when runtime, cockpit assets, or schema boundaries require it; auto-restart only when explicitly enabled and no active work or only safely paused work exists.
- Stop every runtime process you started before final reporting unless Devon asked to keep it running.
- Remove temporary probe files and fixtures as you go; never delete durable `~/.heiwa/state` evidence without explicit approval.

GitHub plus Cloudflare are the public install source: GitHub owns source, releases, checksums, and CI evidence; Cloudflare may front docs, install pages, update manifests, and status, but must not become a second binary authority.

Heiwa must initialize and adapt per machine through `~/.heiwa/machine.json`; do not hardcode one-user or one-device assumptions into runtime behavior.

Promotion rule (experimental -> integration -> production): work starts on a short-lived experimental branch from current `dev` (`codex/*` for Codex; use the provider's equivalent prefix). All agent commits land there first; direct commits and pushes to `dev` or `main` are forbidden. Run `HEIWA_BRANCH_MODE=experimental bash scripts/check_ci_local.sh`, open an experimental -> `dev` pull request, resolve review and CI, then merge. `dev` is the protected integration branch and must never be behind `main`; at integration and promotion handoffs it must contain at least one verified, value-bearing commit beyond `main`. Equality is allowed only during the brief post-promotion synchronization window — use `HEIWA_BRANCH_MODE=post-promotion bash scripts/check_agent_baseline.sh` — and the next accepted experimental merge restores the ahead invariant. Never create empty/sentinel commits merely to alter the count. Update production only via a `dev` -> `main` pull request after `bash scripts/check_ci_local.sh` passes on `dev`. GitHub branch protection on both branches (`enforce_admins`, required status checks, 0 approvals — self-merge is fine) blocks direct pushes; do not bypass it. Do not hold a standing `dev` -> `main` PR open between promotions. After `main` updates, promote to the installed `heiwa` runtime via local checkout updates (`heiwa app update --source checkout`).

CI economy: required PR CI runs two lanes. The feedback lane (secret/vulnerability scan, dependency review, web lint, docs, agent sync, repo hygiene) is a hard sub-minute gate at one minute per job. The Rust promotion lane (`Rust Tests`, `Rust Static Checks`) compiles, so it is bounded at 20 minutes per job and warmed by the sccache Actions cache seeded on protected `main`; a typical PR finishes it in a few minutes. Both lanes report through the aggregate `Rust Source Policy` status context, which `main` branch protection pins by name — never rename or delete the job that reports it, or every PR blocks on a check that can never report. Both lanes run on `blacksmith-4vcpu-ubuntu-2404`; `scripts/check_ci_job_deadlines.rb` still rejects unproven runner labels because `timeout-minutes` bounds a running job and not queue time (only `ubuntu-latest` and `blacksmith-4vcpu-ubuntu-2404` are proven to claim jobs here — canary any other label before using it). Every protected-main commit reruns CI plus exhaustive cross-platform certification, and releases require both workflows at the exact commit. Deploy remains dispatch-only. Local verification is the default; remote CI is the promotion gate, not a development loop.

Agent baseline gate: before closing repo-health work, local promotion, or peer-agent handoff, run `bash scripts/check_agent_baseline.sh` on `dev`, or set `HEIWA_BRANCH_MODE=experimental` on an experimental branch. The gate is local-only and must not be treated as remote health. Remote operations (`git fetch/pull/push`, `gh run`, releases, `wrangler deploy`) require an explicit assignment and the remote pre-flight in `docs/agent-baseline-workflow.md`.

Vendor quarantine: root `vendor/` is ignored local research quarantine. `vendor/oss-lifts` is not part of the production remote checkpoint. Do not add, remove, import from, or cite `vendor/` as product evidence unless Devon assigns a tracked-vendor slice with license/provenance and `PRODUCT_SURFACE.md` updates.

## Working Priorities

Every work item must classify as Intake, Execution, Evidence, or out-of-scope (see [Three Planes in `HEIWA.md`](HEIWA.md#the-three-planes)). If it does not advance a plane, defer or reject.

Prioritize this order:

1. local runtime truth
2. provider/account normalization
3. evidence and bounded execution
4. internal backend sync and optional remote support paths
5. remote surfaces such as `/code`

Do not optimize for maturity theater first:

- do not overstate `/code`
- do not over-rotate into web-console-first language
- do not introduce a hosted control plane as the product center
- do not pretend every wrapped provider is equally integrated

## Context Engineering Rules

Treat every coding agent as a fast junior engineer with strong recall and weak
judgment unless the repo gives it guardrails. Do not offshore architecture to the
model.

- Output defaults to `$caveman`: result/action/blocker first, no filler, exact
  paths/commands/errors. Handoffs must include this simple line: `$caveman; repo
  truth first; ideate, build, execute, and ship real value only, regardless of
  slice size; verify; use reasonable workarounds; report only true blockers.`
- Repo truth beats prompt truth. Inspect current code, schemas, commands, tests,
  runtime status, and docs before making architecture claims.
- Keep context narrow. Load only the files, contracts, errors, and tests needed
  for the next slice. When a thread is saturated or polluted, start a clean
  session instead of relying on summary compaction.
- Mirror third-party source locally when it is needed for implementation truth.
  Put source mirrors under ignored `repos/` and reference exact folders in
  prompts. Do not rely on stale web snippets for package internals.
- Prefer existing service modules, crates, reducers, adapters, and runtime
  contracts over new standalone mechanics.
- Keep PRs and patches cohesive and reviewable. Size work by the value boundary,
  not artificial smallness. Split only where each part can deliver independent
  Intake, Execution, or Evidence value without leaving the product incomplete.
- Every substantial output must separate: acquired data, missing data, needed
  data, executable next action, and verification evidence.
- Post-feature review is mandatory for non-trivial changes: inspect the diff for
  duplicated mechanics, broken layer boundaries, missing tests, and evidence
  gaps before opening or updating a PR.

### Service Layer Rule

Do not generate inline runtime mechanics or duplicate existing database, API,
provider, routing, approval, or evidence interactions. Isolate repeated mechanics
behind reusable service-layer modules. Keep command handlers, routes, reducers,
and UI actions thin and responsible for domain policy and presentation. Search
for existing patterns before creating new files.

### Stack Selection Rule

Choose tools by contextual legibility, local-first authority, and runtime fit.
For Heiwa, this currently means:

- Rust for runtime authority, provider supervision, local execution, leases, and
  evidence.
- Local JSONL for text truth; Lance for local recall over the evidence corpus; GitHub sync remains planned and redaction-gated.
- TypeScript for client contracts and companion app surfaces.
- Shell for install, bootstrap, and operator glue.

Do not replace this spine with a dashboard-first or hosted-backend-first stack
unless repo truth and a specific migration plan prove the value.

## Required Reading

Before making architecture or runtime changes, read:

1. `HEIWA.md`
2. this file
3. `docs/local-self-operation.md`
4. `CLAUDE.md` or `GEMINI.md` when working through that provider surface

Before competitor-parity, product-positioning, memory, gateway, desktop app, or
connector work, also read `docs/research/competitive-landscape-2026-05.md` and
refresh live Hermes/OpenHuman facts before citing stars, releases, integration
counts, or feature parity.

## Shared Peer Truth

Apply this across Codex, Claude, Gemini, Grok, and Heiwa docs:

- Do not cite Hermes as a worker mesh. Cite it for learning loop, skills,
  FTS5 recall, Honcho user modeling, messaging gateway, cron delivery, MCP,
  provider/model switching, and terminal backends.
- Do not call OpenHuman pure local-first. Its README says local memory plus
  managed default services for sign-in, routing, search proxying, OAuth, and
  Composio-backed integrations.
- Do not claim Tauri 2 is peer-validated by OpenHuman. OpenHuman uses vendored
  Tauri/CEF sources. Heiwa chooses Tauri 2 because it fits Rust + Solid/Vite +
  local runtime authority, and must prove plain WebView is enough.
- Biggest current peer gap: connector setup and tool breadth. OpenHuman claims
  118+ integrations through Composio; Hermes claims 40+ tools plus MCP.
- Second peer gap: compression/learning loop. OpenHuman claims TokenJuice;
  Hermes ships skill self-improvement. Heiwa has local read models and static
  docs today, not equivalent loops.

If repo docs drift, `HEIWA.md` is the canonical architecture file.
