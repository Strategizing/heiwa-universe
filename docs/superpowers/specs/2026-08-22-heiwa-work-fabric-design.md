# Heiwa Work Fabric Design

Date: 2026-08-22
Status: approved architecture; implementation planning not started
Scope: Heiwa.app, local runtime, multi-repository work, provider agents, productivity context, GitHub collaboration, approvals, and evidence
Planes: Intake, Execution, Evidence

## Decision

Build Heiwa as a local-first work operating system on the existing Rust, Tauri,
TypeScript, operator-stream, DREX, approval, and evidence spine.

Do not build a generic desktop framework, a replacement for Tauri, a new Git
forge, a hosted control plane, or a provider-neutral inference proxy. Build one
integrated first-party product that joins:

- one human/Heiwa conversation;
- mail, calendar, files, browser, GitHub, and machine signals;
- multiple repositories, worktrees, terminals, sessions, and agents;
- the user's local models and provider-owned subscription or API runtimes;
- exact approvals, artifacts, verification, and receipts.

The durable user-facing unit is a **Work Session**. A Work Session is a
read-only projection over the existing operator thread and associated runtime
state. It may span multiple repositories and provider sessions without creating
a second write authority.

Compression:

> Signals and intent become one governed Work Session; DREX routes work across
> real agents and tools; Heiwa shows, verifies, publishes, and proves the result.

## Product Promise

A fresh user can install Heiwa, connect one inference source, GitHub, and one
productivity ecosystem, then complete this workflow in one application:

1. A real mail, calendar, file, or direct user request becomes work.
2. Heiwa opens the relevant multi-repository workspace.
3. DREX routes calls across eligible local and provider-owned inference.
4. Workers operate in isolated worktrees and visible terminal panes.
5. The user inspects progress, files, diffs, tests, artifacts, and blockers.
6. Risky actions become exact staged approvals.
7. Heiwa publishes an approved branch or draft pull request and records the
   external receipt.
8. The session survives navigation, restart, and compatible updates.

Home, Work, and Agent surfaces are projections of the same state. They must
never disagree about what is running, blocked, approved, changed, or complete.

## Goals

- Make a fresh install empty, honest, actionable, and independent of maintainer
  state.
- Support multiple repositories, worktrees, sessions, terminals, agents, and
  files in one window.
- Use provider-owned authentication and inference while letting DREX select the
  cheapest eligible source above the call's quality, privacy, and capability
  floors.
- Attach productivity context without copying entire external accounts into
  Heiwa state.
- Keep GitHub canonical for source, branches, pull requests, checks, releases,
  and public trust.
- Make every worker, approval, mutation, artifact, and receipt attributable and
  inspectable.
- Recover honestly from crashes, stale state, corruption, incompatible schemas,
  provider failure, and uncertain external effects.
- Prove the internal contracts across first-party surfaces before exposing an
  extension framework.

## Non-Goals

- Hosting source code or replacing GitHub.
- Owning provider prompts, authentication, quota semantics, model inventory, or
  inference internals.
- Pooling or redistributing subscription credentials.
- Synchronizing raw operator journals, private productivity data, secrets, or
  the Lance index through GitHub.
- Letting TypeScript surfaces own policy, persistence, routing, approvals, or
  shell execution.
- Treating a terminal pane, discovered executable, or provider account as a
  verified worker merely because it exists.
- Shipping third-party surface or connector APIs before first-party contract
  stability is demonstrated.
- Starting mesh pairing, transport, relay, or replication as part of this
  program. Existing machine and node identity may be referenced, but mesh
  delivery remains independently gated.

## Peer Lessons

Heiwa borrows product patterns without inheriting peer ownership models.

| Reference                 | Pattern Heiwa adopts                                                 | Boundary Heiwa keeps                                                               |
| ------------------------- | -------------------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| Cursor Origin             | Repository, pull-request, review, check, and GitHub projections      | GitHub remains source authority; Heiwa does not become a forge                     |
| Cursor Agents             | Multi-root workspaces, isolated worktrees, parallel sessions         | Local-first execution remains viable without hosted agents                         |
| herdr                     | Persistent real terminal panes, agent visibility, detach/reattach    | Heiwa owns the durable terminal and worker contract                                |
| Coder Mux                 | Isolated local/worktree/SSH workspaces and Git divergence visibility | Heiwa does not adopt a second agent loop or state authority                        |
| ChatGPT/Claude-style apps | Conversation, artifacts, connectors, approvals, projects             | Heiwa joins these to local execution and evidence rather than copying their shells |

Primary references:

- Cursor Origin: <https://cursor.com/changelog/origin-code-hosting>
- Cursor worktrees: <https://cursor.com/docs/configuration/worktrees>
- Cursor multi-root workspaces: <https://cursor.com/changelog/04-24-26>
- herdr: <https://github.com/motionharvest/herdr>
- Coder Mux: <https://github.com/coder/cmux>

## Existing Substrate

This design extends current Heiwa architecture instead of introducing a
parallel framework:

- `heiwa_shell` remains the local runtime and authenticated API host.
- DREX remains the routing, capability, budget, and execution kernel.
- `heiwa_session::OperatorSessionService` remains the sole operator-domain
  writer.
- `heiwa_evidence` remains the append/replay and sensitive-material boundary.
- Local JSONL remains durable evidence truth; Lance remains derived recall.
- Tauri remains the narrow native and OS-integration layer.
- TypeScript/Solid surfaces remain disposable projections and typed intent
  producers.
- Existing approvals, artifacts, receipts, provider adapters, worker leases,
  machine identity, node identity, and herdr pane bridges are starting seams.

No surface may import another surface to mutate its state. No command handler,
connector, or renderer path may create a second approval, evidence, session, or
provider-routing authority.

## Architecture

### Integrated Data Flow

```text
composer + passive signals
          |
          v
attention + bounded context references
          |
          v
Work Session + multi-repository Workspace
          |
          v
DREX plan, route, leases, and budgets
          |
          v
workers + terminals + tools + connectors
          |
          v
staged actions + exact approvals
          |
          v
artifacts + verification + external receipts
          |
          v
Home / Work / Agent projections
```

The operator stream remains the ordered state transition log for the session.
Large source material remains behind bounded references. Projections may be
discarded and rebuilt.

### Language and Ownership Boundaries

**Rust owns:**

- state machines and durable domain events;
- workspace and repository policy;
- provider routing, execution, budgets, leases, and cancellation;
- terminal and worker authority;
- context eligibility and privacy decisions;
- staged actions, approvals, execution, reconciliation, and receipts;
- compatibility, recovery, and evidence.

**Tauri owns:**

- authenticated loopback transport below the renderer;
- runtime supervision and packaged sidecars;
- windows, tray, shortcuts, notifications, clipboard, file picker, and safe OS
  context;
- secure-store access without returning secrets to TypeScript.

**TypeScript/Solid owns:**

- rendering read models;
- local, disposable interaction state;
- typed user intents;
- layout, panes, previews, inspectors, navigation, and accessibility.

TypeScript does not persist product truth, route inference, execute arbitrary
commands, call connectors directly, or decide whether an action is approved.

## Core Domain Contracts

### Work Session

`WorkSessionSnapshotV1` is a read-only projection over one operator thread. It
contains:

- `session`: thread ID, title, objective, optional project reference, derived
  phase, timestamps, and replay cursor;
- `attention[]`: severity, deterministic reason, source references, and
  suggested typed intent;
- `context[]`: reference, kind, source, title, freshness, sensitivity,
  permission, and bounded summary;
- `workspace`: roots, repositories, worktrees, panes, files, previews, and
  remote collaboration state;
- `turns[]`: user and Heiwa transcript projections;
- `runs[]`: worker, provider, model, pane, state, progress, resource, cost-truth,
  and evidence references;
- `actions[]`: target, payload summary and hash, risk, approval state, execution
  state, and idempotency key;
- `artifacts[]`: kind, title, location, producer, source spans, and verification
  state;
- `receipts[]`: local or external result identity and evidence references;
- `blockers[]`: exact missing authority, capability, data, or external state;
- `compatibility`: skipped future records, corrupt inputs, degraded
  projections, and required upgrade state.

Mail bodies, file contents, browser state, terminal logs, and repository blobs
are not duplicated into the snapshot. They remain source-owned and are loaded
through bounded references when authorized.

### Workspace

`WorkspaceSnapshotV1` contains:

- workspace ID and human label;
- one or more canonical local roots;
- repository identity, remote, default branch, current branch, HEAD, dirty
  state, upstream divergence, and pull-request/check projections;
- worktree identity, owning worker/session, base commit, and mutation lease;
- tabs, panes, provider sessions, processes, previews, and focused resources;
- repository and filesystem permissions;
- synchronization mode and any active publication lease.

One Work Session may span multiple repositories. Repository lanes can execute
concurrently only when their inputs and mutation scopes are independent.

### Surface Definition

The existing `SurfaceModule` evolves internally into `SurfaceDefinition`:

- identity: ID, label, icon;
- placements: Home card, full view, side pane, hover preview;
- requirements: read models and capabilities;
- commands: typed intents;
- risk: possible action classes;
- empty states: missing, disconnected, unavailable, incompatible, and truly
  empty;
- presentation: summaries, inspectors, timelines, diffs, and artifacts;
- contract tests: rendering, permissions, action staging, and compatibility.

All first-party surfaces consume the shared Work Session store.

## Rust Services

### Work Session Projector

Folds operator events, worker leases, terminal state, approvals, artifacts,
receipts, connector references, and repository projections into
`WorkSessionSnapshotV1`. It is read-only and replayable.

### Attention Engine

Produces deterministic Home priorities from user commitments, connector
signals, runtime failures, deadlines, approvals, and active work. A model may
summarize an attention item but cannot silently change its priority or risk.

### Context Broker

Selects bounded references from Calendar, Mail, Files, Browser, memory, and
projects. Every reference carries source, freshness, sensitivity, permission,
and provider eligibility. Connection does not imply permission to send raw data
to every provider.

### Workspace Coordinator

Owns repository discovery, canonical roots, multi-root membership, worktree
creation, writer leases, dependency ordering, dirty-tree preservation,
conflict detection, and repository reconciliation.

### Execution Coordinator

Extends the existing operator turn runner and DREX path. It plans, routes,
launches, supervises, cancels, and records provider calls, workers, terminal
sessions, and tool execution without creating another agent loop.

### Terminal Runtime

Defines a Heiwa-owned contract for:

- workspace, tab, pane, process, and provider-session identity;
- create, attach, restore, read, send, split, focus, pause, resume, and stop;
- verified state: working, waiting, blocked, failed, done, stale, or unverified;
- cwd, repository, branch, lease, resources, and evidence references.

herdr is the current adapter. `heiwa_session` is the durable target. Renderer
code consumes the Heiwa contract and never shells out to herdr directly.

### Action Gate

The existing approval executor becomes the sole path for governed effects.
Surfaces and workers submit typed action intents. Action Gate stages, approves,
revalidates, executes, reconciles, and receipts them.

### Evidence and Artifact Projector

Links outputs, diffs, documents, screenshots, source spans, external IDs,
verification, and receipts to their producing work. It never turns a summary
into canonical evidence.

### GitHub Collaboration Service

Projects repository metadata, divergence, branches, pull requests, reviews,
and checks. It stages remote writes through Action Gate. It does not host code
or accept private runtime evidence as repository content.

## GitHub Synchronization

GitHub remains canonical for user source repositories.

Synchronization modes:

1. **Observe** — automatically refresh remote metadata, branches, pull requests,
   reviews, and checks. No remote writes.
2. **Review each publish** — default. Every branch push or pull-request mutation
   becomes an exact staged action.
3. **Auto-sync Heiwa branches** — optional, explicit, revocable publication
   lease bounded by repository, Heiwa-owned branch prefix, operations, actor,
   and expiry.

No synchronization mode permits force-push, direct default-branch mutation,
merge, release, destructive remote action, or publication of untracked private
material without separate authority.

Every remote mutation records the repository, remote, commit SHA, branch,
payload hash, approval or publication lease, actor, result ID, and timestamp.

Source synchronization and evidence synchronization are different systems.
Raw operator journals, private connector data, secrets, and Lance indexes remain
local. Future redacted evidence sync retains its independent privacy and schema
gate.

## Inference Federation

Each local model, provider CLI, and direct-API account is a capability source.

- Providers own authentication, model inventory, prompts, session semantics,
  quota, and inference internals.
- Heiwa owns discovery, account-health projections, per-call admission,
  routing, privacy, leases, budgets, context, execution evidence, and UX.
- DREX selects the cheapest eligible candidate above the call's capability,
  quality, privacy, success, locality, and budget floors.
- A thread or worker is not permanently assigned one model. Routing remains per
  call.
- Local inference handles cheap or sovereign work when capable.
- Subscription CLIs are used only through their supported provider-owned auth
  and runtime surfaces.
- Heiwa does not scrape credentials, disguise API use as subscription use, or
  claim quota truth it cannot observe.

Provider state and cost retain explicit truth classes such as measured,
provider-reported, estimated, defaulted, and unknown.

## Productivity Context

Calendar, Mail, Files, Browser, and later connectors are session-attached
capabilities with focused inspector views. They do not become separate state or
execution authorities.

Every first-party connector must implement:

- connect and revoke;
- account and permission health;
- list or query bounded items;
- return source references with freshness and sensitivity;
- stage at least one useful typed action;
- execute only through Action Gate;
- return an external receipt or explicit uncertain result;
- expose missing, disconnected, unavailable, incompatible, and empty states.

Passive connector signals may create attention items and summaries. They do not
silently send messages, alter calendars, publish code, or cause other risky
effects.

## User Experience

### Fresh Install

Heiwa opens empty but operational:

- create the per-user runtime root and real machine manifest;
- show sourced machine facts;
- show no maintainer data, example accounts, fake sessions, fake workers, or
  demo receipts;
- discover provider and machine capabilities without mutating provider-owned
  configuration;
- guide the user through connecting inference, GitHub, productivity accounts,
  and local folders progressively.

Disconnected optional capabilities do not prevent local work. Missing, corrupt,
future-schema, and secure-storage failures render differently.

`device_id` remains a random local installation handle. The mesh `node_id`
remains a public-key fingerprint. The UUID is never promoted into cryptographic
node identity.

### Surface Roles

**Home** answers what matters, what is working, what needs the user, and what
recently completed.

**Work** shows conversation, objective, context, repositories, plan, progress,
files, diffs, tests, staged actions, artifacts, and receipts.

**Agent** shows workers, provider sessions, terminals, panes, worktrees, routes,
tools, budgets, resources, failures, and take-over controls.

**Approvals** is a shared interruption layer available from every relevant
projection.

Home groups state as Needs You, Working, and Recent. Opening an item restores
the same Work Session and workspace rather than navigating to a disconnected
product silo.

## Lifecycle

### Create

Direct intent or a promoted attention item creates an operator thread and Work
Session projection with objective, acceptance criteria, workspace, bounded
context, policy, and replay cursor.

### Prepare

Workspace Coordinator snapshots repository HEADs, dirty state, branches,
remotes, worktrees, pull requests, and upstream divergence. It preserves user
changes, creates isolated worktrees for mutation, reserves scopes, and provides
the facts DREX needs to build a visible plan.

### Execute

Each worker receives an exact objective, repository scope, worktree or read-only
root, provider-owned session, tool and network leases, budget, cancellation
boundary, and verification contract. Independent lanes may run concurrently;
dependent lanes remain ordered.

### Review

Before publication or another external effect, Work shows changed repositories,
files, full diff, tests, skipped checks, privacy/secret scan, target, payload,
risk, expected receipt, and known uncertainty.

### Publish

An approved publication refreshes remote state, stops on meaningful drift,
runs required gates against the exact commit, pushes the authorized branch,
opens or updates the draft pull request, reads resulting checks, and records the
external receipt.

### Recover

On restart or update, Heiwa replays durable events, reattaches only matching
live sessions, revokes or closes unproven leases, reconciles uncertain external
actions by external identity, preserves worktrees and user files, and never
silently resurrects or repeats risky work.

### Complete

A Work Session is complete only when its acceptance criteria are satisfied and
verified. A stopped agent, local commit, pushed branch, or open pull request is
an intermediate fact unless it is the accepted outcome.

State meanings:

- `degraded`: optional capability unavailable; useful work continues;
- `blocked`: required authority, data, decision, capability, or external state
  remains missing after safe alternatives are exhausted;
- `failed`: execution ended without satisfying its contract;
- `uncertain`: an external effect may have occurred and must be reconciled;
- `cancelled`: authority was withdrawn; partial artifacts remain visible;
- `incompatible`: required durable state cannot be safely interpreted;
- `complete`: acceptance criteria are verified.

## Security and Authority

### Trust Boundaries

Human operator, native bridge, renderer, local runtime, worker process,
provider, connector, GitHub remote, and future mesh node are distinct
principals.

Loopback is transport, not trust. Native requests retain the existing signed
request contract binding method, numeric port, exact target, body digest,
timestamp, and single-use nonce. Browser preview retains single-use bootstrap
and HttpOnly local session behavior. Renderer code never receives secret
material.

### Legitimate Workers

A verified worker records:

- worker ID and parent;
- provider and non-secret provider-session reference;
- workspace, repositories, worktrees, and canonical roots;
- executable identity and execution location;
- tool, filesystem, network, budget, action, and expiration leases;
- start time, heartbeat, process/session state, artifacts, and evidence.

UI distinguishes verified live, unverified terminal, stale, revoked, failed,
and complete. Workers cannot approve their own actions, widen leases, claim the
human actor, or transfer authority to child workers implicitly.

### Legitimate Approvals

Approval binds:

- approving actor and device;
- action type and exact target;
- exact payload or content hash;
- human-readable preview;
- workspace and repository scope;
- risk, cost, and expected receipt;
- policy version, creation, expiry, and idempotency key.

Any target, recipient, branch, payload, cost, risk, or policy change invalidates
the approval. Action Gate revalidates approval, content, leases, permissions,
drift, and external state immediately before execution. Approval is consumed
once.

Publication leases are bounded approvals, not hidden global switches.

### Risk Defaults

| Class                    | Examples                                                                    | Default                                     |
| ------------------------ | --------------------------------------------------------------------------- | ------------------------------------------- |
| Observe                  | Read files, repository status, remote metadata, bounded connector summaries | Automatic inside granted scopes             |
| Isolated mutation        | Edit, test, and commit inside a Heiwa-owned worktree                        | Automatic with worker lease                 |
| External write           | Push branch, open pull request, send mail, change calendar                  | Exact approval or bounded publication lease |
| Sensitive or destructive | Merge, release, delete, secrets, finance, identity                          | Explicit action approval                    |
| Forbidden                | Policy bypass, impersonation, secret publication                            | Refuse                                      |

### Privacy and Isolation

- Provider, GitHub, and connector secrets remain in provider stores or OS secure
  storage.
- Workers receive only task-required environment values; children do not
  inherit the whole runtime environment.
- Canonical roots, symlink checks, worktrees, and writer leases prevent path
  escape and overlapping mutation.
- Raw terminal commands remain an explicit advanced capability.
- Untrusted code requires the configured sandbox lane and never executes in a
  credential-bearing host context.
- Context Broker enforces sensitivity, locality, provider eligibility, and user
  policy on every model call.
- Sensitive values are rejected from evidence, logs, artifacts, Lance, and
  GitHub rather than relying on later cleanup.

## Error and Compatibility Contract

Every surfaced error includes a stable code, responsible boundary, safe-retry
status, acquired data, missing data, possible side-effect state, required user
action, and evidence or reconciliation reference.

Safe routing may work around unavailable providers, connectors, or optional
context. It may not route around approval, privacy, root, lease, or integrity
boundaries.

Persistence behavior is strict:

- missing record means unconfigured or empty;
- corrupt record means repair or recovery state;
- future schema means compatibility state and upgrade guidance;
- secure-storage failure means credential backend failure;
- partial journal corruption preserves valid records, counts damage, and marks
  degradation;
- policy and persistence boundaries do not silently convert errors to absence.

Refresh and display paths use the same strict loaders.

Durable domains use versioned envelopes. Readers skip optional future events,
count them, and surface degradation. Unknown required state never renders as
empty. Writers never reinterpret or rewrite unknown future records. Unsafe
downgrades enter read-only recovery mode. Migrations are atomic and preserve a
recoverable prior state.

## Verification

### Defining Integrated Acceptance

The program's public-ready test is one fresh-user chain:

1. install a certified empty build;
2. connect provider-owned inference, GitHub, and one productivity ecosystem;
3. open a workspace containing at least two repositories;
4. promote a real productivity signal or submit direct intent;
5. route work across eligible local and subscription inference;
6. execute in isolated worktrees and visible panes;
7. show identical Home, Work, and Agent truth;
8. produce diffs, tests, artifacts, staged actions, and exact approval;
9. publish an authorized branch and draft pull request;
10. record commit, pull request, checks, and productivity follow-up receipts;
11. restart or update and restore the session without data loss, duplicated
    effects, or false completion.

### Acceptance Matrix

| Area               | Required proof                                                                    |
| ------------------ | --------------------------------------------------------------------------------- |
| Fresh install      | No phantom accounts, sessions, workers, connectors, receipts, or maintainer paths |
| Inference          | Provider auth remains provider-owned; unavailable candidates reroute honestly     |
| Multi-repository   | One session changes at least two repositories through isolated worktrees          |
| Workers            | Every verified worker has identity, lease, liveness, scope, and outcome           |
| Multiplexer        | Panes support navigation and documented restart/reattach behavior                 |
| Shared projections | Home, Work, and Agent agree from one event cursor                                 |
| Approvals          | Modified, expired, replayed, stale, and self-approved actions cannot execute      |
| GitHub             | Drift, branch publication, draft pull request, checks, and receipts work          |
| Productivity       | Connect, read, bounded context, staged write, execute, receipt, and revoke work   |
| Recovery           | Crash injection does not lose local truth or duplicate uncertain effects          |
| Compatibility      | Missing, corrupt, future, and incompatible state remain distinguishable           |
| Privacy            | Renderer, logs, evidence, artifacts, and GitHub contain no raw secrets            |
| Update             | Exact version transition preserves active and completed work honestly             |
| Multi-user         | Disposable user roots never share state, auth, paths, or receipts                 |

### Test Layers

1. Contract tests for snapshots, workers, workspaces, actions, approvals,
   artifacts, receipts, connectors, and schema negotiation.
2. State-machine tests for replay, cancellation, restart, expired leases,
   concurrent writers, partial corruption, future schemas, and uncertain
   effects.
3. Adapter contract suites using fake provider CLIs, models, Git remotes,
   GitHub, productivity services, terminal adapters, and secure storage.
4. Desktop tests for fresh install, onboarding, projection agreement, panes,
   diff review, approval invalidation, degraded states, keyboard use, and
   accessibility.
5. Packaged Tauri acceptance against disposable state and a temporary checkout
   runtime port. Browser preview is not packaged-app proof.
6. Opt-in account-backed smoke tests against authorized provider, GitHub, and
   productivity test accounts before public connector certification.
7. Adversarial tests for secret and prompt injection, symlink escape, malicious
   repositories, forged workers, replayed approvals, renderer compromise,
   uncertain network completion, and hostile schema data.

All automated tests use disposable configuration and evidence roots. They do
not depend on maintainer accounts, warm state, existing panes, or installed
runtime data.

## Promotion and Release

```text
approved spec
  -> implementation on dev
  -> contract and integration gates
  -> disposable checkout runtime on 7475
  -> packaged-app acceptance
  -> clean agent baseline
  -> dev-to-main pull request
  -> SHA-bound protected CI
  -> certified release from exact main SHA
  -> clean-machine install/update
  -> installed runtime verification
  -> release receipt
```

Required local gates include:

- `bash scripts/check_ci_local.sh`;
- `bash scripts/check_agent_baseline.sh`;
- `bash scripts/check_ci_local.sh --full` when Lance or its feature boundary
  changes;
- runtime, security, release-metadata, installer, and Desktop acceptance gates
  carried by the canonical scripts.

Every pass is bound to the exact commit tested. Any subsequent commit reruns the
required gates.

Port `7474` remains installed product runtime. Checkout acceptance uses `7475`
or another temporary port, stops processes it starts, and does not mutate
installed state. Strangers install the latest certified GitHub Release built
from protected `main`, not a moving source checkout. Development checkout
promotion records its exact commit and is not public-release evidence.

## Vertical Release Program

This design is too large for one implementation plan. Work is split by usable
outcomes, not artificial patch size. Every release includes user experience,
runtime authority, failure states, approvals, tests, installed-runtime proof,
and evidence.

### Release A — Local Multi-Repository Workbench

Fresh-install truth, Work Session and Workspace projections, multiple local
repositories, isolated worktrees, provider-owned agents, terminal panes,
Home/Work/Agent agreement, diffs, tests, local artifacts, approvals, receipts,
and restart recovery.

This is independently useful for governed local development.

### Release B — GitHub Collaboration

Repository-scoped GitHub connection, remote divergence, branches, pull
requests, reviews, checks, review-each-publication, bounded Heiwa-branch sync,
and branch/pull-request/check receipts.

This provides an Origin-like collaboration surface while GitHub stays
canonical.

### Release C — Productivity Fusion

First-party Calendar, Mail, and Files contracts with connect, health, bounded
read, context reference, staged action, Action Gate execution, receipt, revoke,
and honest failure behavior. One account ecosystem is certified end to end
before additional ecosystems implement the same contract.

Each connector ships only with a real read-to-action outcome.

### Release D — Tandem Operating Loop

Certify the integrated acceptance workflow from productivity signal through
multi-repository/provider execution, exact approval, GitHub result,
productivity follow-up, restart, and receipt.

This is the public-alpha threshold for the broader work operating system.

### Release E — Internal Framework Extraction

After Releases A through D prove identical contracts across at least three
first-party surfaces, extract stable `SurfaceDefinition`, intent, event,
permission, empty-state, compatibility, and test-harness APIs. Remove
first-party special cases. Keep extension loading closed.

### Release F — Third-Party Extension Surface

Only after the internal contracts survive real use and one compatibility
migration, add signed packages, capability manifests, sandboxed execution,
explicit permissions, revocation, upgrades, and connector/surface SDKs.

No generic framework work precedes the integrated product.

## Definition of Done

This architecture program is complete only when:

- the integrated acceptance workflow passes from a clean install;
- all acceptance-matrix proofs exist at one exact protected-main commit;
- a certified release from that commit installs and updates on supported
  platforms;
- no test or user flow depends on maintainer state;
- no surface or adapter bypasses the operator stream, DREX, Action Gate,
  evidence, or secure-storage boundaries;
- GitHub source sync and private evidence remain separated;
- worker and approval legitimacy survive crash, replay, drift, and adversarial
  testing;
- third-party extension APIs remain closed until the first-party extraction
  gate is satisfied;
- the release receipt connects decision, implementation, tests, installed
  runtime, exact commit, release artifact, and public install authority.
