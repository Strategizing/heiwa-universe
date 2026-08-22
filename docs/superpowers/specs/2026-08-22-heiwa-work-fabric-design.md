# Heiwa Work Fabric Design

Date: 2026-08-22
Status: Draft for review — architecture approved in conversation; written artifact approval pending
Scope: Heiwa.app, local runtime, multi-repository work, provider agents, productivity context, GitHub collaboration, approvals, and evidence
Planes: Intake, Execution, Evidence
Supersedes: `2026-08-14-heiwa-app-product-roadmap-design.md` post-L3 sequencing and standalone placement of § L4; preserves its L4 browser ownership and safety requirements as Release D
Preserves: accepted L0-L2 contracts and gates; `2026-08-18-L3-calendar-mail-connectors.md` as connector implementation authority; `2026-08-20-heiwa-mesh-runtime-design.md` as `Work`, node, and L5 authority

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

The durable coordination unit is **Work**, identified by `work_id` as defined by
the mesh design. A **Work Session** is the user-facing, read-only projection of
that Work, its operator threads, workspace, provider sessions, actions, and
evidence. It may span multiple repositories, nodes, and provider sessions
without making thread identity or a UI snapshot a second write authority.

## Precedence and Roadmap Reconciliation

This document is the broad product-sequencing authority after L3. It does not
erase already accepted or implemented layers:

- L0-L2 remain accepted prerequisites with their existing acceptance scripts
  and SHA-bound stamps.
- L3 remains governed by its connector spec and live ledger. The Mac-first
  Apple Calendar lane is already complete through read, T2 approval, live
  external write, receipt, and journal replay. Google Calendar remains blocked
  on external account setup; `gmail.send` remains pending.
- The roadmap's L4 product boundary is absorbed into Release D: a runtime-owned
  browser service, Chromium sidecar, CDP target registry, dedicated Heiwa
  profile, node-aware tab ownership, DREX/Action Gate policy, and receipts. The
  specific screencast and packaging mechanism must pass the Release D spike and
  may change only through an explicit spec amendment.
- The mesh design remains authoritative for durable `Work`, `work_id`, node
  identity, provider-session host affinity, control leases, replication, and
  L5. This document defines how the local runtime creates and projects Work
  before mesh transport exists.

Where documents overlap, implementation details already proven at HEAD win
over greenfield wording here. This spec changes sequencing and composition; it
does not authorize rebuilding accepted capability.

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
- L0-L2 acceptance gates are real and SHA-bound. They remain prerequisites,
  not checks this program may replace.
- L3's Mac-first Apple Calendar lane already supplies executable connector,
  approval, `work_id`, external-ID, receipt, and replay mechanics. Productivity
  releases extend that lane instead of creating another connector framework.
- `MeshEnvelope.work_id` exists as an optional field because some mesh events
  are not Work-scoped, but no general local `Work` producer exists yet. Release
  A1 closes that producer gap.

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

### Work, Thread, and Work Session Identity

`Work` is the durable coordination aggregate. `work_id` is its stable primary
identity across threads, tasks, repositories, provider sessions, nodes, and
surfaces. `Work.revision` is monotonic; every mutating command supplies its
expected revision, and stale writers must reload or deliberately replan.

Fresh local work cannot depend on mesh enrollment. Before a node key exists,
Work records `origin_installation_id`; `origin_node` and `coordinator_node` are
absent, and the Work is ineligible for mesh replication. Enrollment appends a
signed `work_node_bound` event that adds the cryptographic node identity without
changing `work_id` or rewriting history. `device_id` is never used as node
identity. At the replication boundary, the mesh design's required node fields
apply and unbound Work is refused.

The local runtime persists Work as versioned operator-domain events through
`OperatorSessionService`, preserving one local domain writer. The Work
projector folds those events into the aggregate described by the mesh spec.
Mesh replication may later carry those events, but it does not introduce a
second local writer.

An operator thread is an interaction resource attached to Work, not Work's
identity. V1 creates one primary thread atomically with `work_created`; the
schema also carries `related_thread_ids` so review, handoff, or channel-specific
threads can be added later without changing `work_id`.

Migration is append-only:

- new Work creates `work_id` before tasks, connector actions, Work-scoped
  browser control events, or outcome-scoped mesh envelopes are emitted;
- new `Task.context_id` values equal `work_id`;
- an existing thread receives one durable `work_linked` event with a generated
  `work_id` when first promoted into Work;
- when existing thread/task/connector evidence already carries one consistent,
  valid `work_id`, migration adopts that ID after collision checks. Conflicting
  historical IDs produce an explicit migration conflict and are never silently
  merged;
- historical task or evidence rows are not rewritten. A migration projection
  retains their legacy context and links it to the new Work;
- L3/L4 domain events associated with a user outcome require `work_id`.
  Capability, peer-health, and other system-wide events may remain unscoped,
  which is why `MeshEnvelope.work_id` stays optional.

`WorkSessionSnapshotV1` is a read-only projection keyed by `work_id`. It
contains:

- `work`: work ID, Work revision, intent, status, origin installation, optional
  origin/coordinator nodes, timestamps, and optional project reference;
- `interaction`: primary thread ID, related thread IDs, derived phase, and
  operator replay cursor;
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

### Snapshot and Delta Delivery

The snapshot is a bounded baseline, not a payload resent after every event. It
carries:

- `work_revision`: durable Work aggregate revision;
- `projection_revision`: monotonic revision of the materialized read model;
- `operator_cursor`: durable operator-stream replay boundary;
- source watermarks for repository, terminal, GitHub, and connector
  projections;
- pagination tokens or summary counts for large collections.

After baseline load, the authenticated operator WebSocket carries
`WorkSessionDeltaV1` envelopes derived from the existing operator stream:

```text
WorkSessionDeltaV1 {
  work_id,
  base_projection_revision,
  projection_revision,
  operator_cursor,
  upserts: { family -> rows keyed by stable id },
  removals: { family -> stable ids },
  source_watermarks,
  compatibility,
}
```

Durable changes advance the operator cursor. Transient provider tokens,
terminal progress, and resource samples use disposable signal frames and never
claim durable revision. Clients apply a delta only when
`base_projection_revision` matches their current projection. A gap, invalid
cursor, unknown required schema, or backpressure overflow yields
`resync_required`; the client discards only its disposable projection and
fetches a fresh snapshot.

Collections are stable-ID keyed, bounded, and paginated. Full terminal logs,
file bodies, diffs, browser frames, and connector bodies load through separate
authorized detail endpoints. This prevents Home from refetching or retaining
an unbounded Work graph.

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

### Cross-Repository Task Graph

Dependency ordering is explicit in `WorkTaskGraphV1`, not inferred from pane
activity or task text:

- each task names `task_id`, `work_id`, repository and path read/write scopes,
  expected base commits, acceptance criteria, and required artifacts;
- directed `depends_on` edges form a validated acyclic graph;
- barrier nodes model integration, combined verification, publication, or a
  user decision across repository lanes;
- the scheduler topologically admits ready tasks and refuses concurrent tasks
  whose write scopes overlap;
- a failed required dependency blocks descendants while independent lanes may
  continue;
- replanning creates a new graph and increments `Work.revision`; it never
  mutates the accepted graph invisibly;
- every task result records the graph revision it satisfied.

Cross-repository publication is an ordered saga, not an atomic transaction.
Each repository commit, branch push, pull request, check, and follow-up action
has its own idempotency key and receipt. Partial completion is explicit; Heiwa
does not claim rollback where GitHub or another service cannot provide it.

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

Folds Work and operator events, worker leases, terminal state, approvals,
artifacts, receipts, connector references, and repository projections into
`WorkSessionSnapshotV1` and `WorkSessionDeltaV1`. It is read-only, bounded, and
replayable.

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
creation, writer leases, `WorkTaskGraphV1` validation and topological
scheduling, dirty-tree preservation, conflict detection, publication sagas,
and repository reconciliation.

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

Calendar, Mail, Files, and later account connectors are session-attached
capabilities with focused inspector views. They do not become separate state or
execution authorities.

This is extension work, not greenfield replacement. The accepted Mac-first
Apple Calendar lane already proves connect/disconnect, bounded read, app-side
staging, immutable T2 approval, live external write, external ID, receipt, and
journal replay. Release C keeps that implementation and closes Mail, Files,
Google Calendar, and additional account breadth through the same contracts.

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

## Browser Work Surface

Release D absorbs the roadmap's L4 browser boundary. `crates/heiwa_browser` is
a runtime service; Tauri and TypeScript only display frames and submit typed
intents. The runtime owns Chromium lifecycle, CDP, target discovery, profile
state, ownership, policy, evidence, and recovery.

The selected product shape is:

- a packaged Chromium sidecar with a dedicated Heiwa user-data directory under
  the per-user configuration root, never the user's system-browser profile;
- raw CDP available only inside the Rust browser service;
- `Page.startScreencast` or a measured replacement delivering bounded frames to
  the app without turning the renderer into browser authority;
- a target registry keyed by node and target identity;
- `Owner::User { node_id }` or `Owner::Agent { session_id }`, with explicit,
  receipted handoff before an agent may control a user-owned target;
- page content treated as untrusted data, never executable instruction;
- credentials usable inside the dedicated profile but never extractable into
  model context, evidence, logs, or renderer state;
- read/extract/screenshot, navigate/open, click/fill/submit, and
  credential/payment/destructive actions mapped through existing risk and
  Action Gate policy.

Before product implementation, Release D runs a bounded cross-platform spike
covering sidecar packaging, frame latency, input fidelity, accessibility,
clipboard, downloads, pop-ups, authentication, crash recovery, and resource
cost. Failure of `Page.startScreencast` to meet the product bar changes only the
display transport through a spec amendment; it does not move browser policy,
profile, ownership, or CDP authority into TypeScript.

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

Direct intent or a promoted attention item atomically creates durable Work,
`work_id`, its primary operator thread, and the first Work Session projection
with objective, acceptance criteria, workspace, bounded context, policy, and
replay cursor.

### Prepare

Workspace Coordinator snapshots repository HEADs, dirty state, branches,
remotes, worktrees, pull requests, and upstream divergence. It preserves user
changes, creates isolated worktrees for mutation, reserves scopes, and provides
the facts DREX needs to build and validate a visible `WorkTaskGraphV1`.

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

Offline repository, GitHub, browser, or connector state remains readable only
with its recorded freshness and source watermark. Heiwa may stage a future
action while offline, but it revalidates Work revision, remote drift,
permissions, approval, and idempotency immediately before execution. It never
blindly retries an `uncertain` external effect.

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
4. create one durable Work whose `work_id` appears across thread, tasks,
   connector events, browser control events, actions, artifacts, and receipts;
5. promote a real productivity signal or submit direct intent;
6. route work across eligible local and subscription inference;
7. execute the validated dependency graph in isolated worktrees and visible
   panes;
8. inspect or act through a node-owned browser target under the same Work;
9. show identical Home, Work, and Agent truth through snapshot plus deltas;
10. produce diffs, tests, artifacts, staged actions, and exact approval;
11. publish an authorized branch and draft pull request;
12. record commit, pull request, checks, and productivity follow-up receipts;
13. restart or update and restore the session without data loss, duplicated
    effects, or false completion.

### Acceptance Matrix

| Area                | Required proof                                                                          |
| ------------------- | --------------------------------------------------------------------------------------- |
| Fresh install       | No phantom accounts, sessions, workers, connectors, receipts, or maintainer paths       |
| Work identity       | One durable `work_id` joins threads, tasks, events, actions, artifacts, and receipts    |
| Projection delivery | Bounded snapshot plus ordered deltas recover from gaps without full-graph polling       |
| Inference           | Provider auth remains provider-owned; unavailable candidates reroute honestly           |
| Multi-repository    | One session changes at least two repositories through isolated worktrees                |
| Dependency graph    | DAG validation, scope-conflict refusal, failure propagation, and revisioned replan work |
| Workers             | Every verified worker has identity, lease, liveness, scope, and outcome                 |
| Multiplexer         | Panes support navigation and documented restart/reattach behavior                       |
| Shared projections  | Home, Work, and Agent agree from one event cursor                                       |
| Approvals           | Modified, expired, replayed, stale, and self-approved actions cannot execute            |
| GitHub              | Drift, branch publication, draft pull request, checks, and receipts work                |
| Productivity        | Connect, read, bounded context, staged write, execute, receipt, and revoke work         |
| Browser             | Node-aware ownership, explicit handoff, hostile-page refusal, and Action Gate work      |
| Recovery            | Crash injection does not lose local truth or duplicate uncertain effects                |
| Compatibility       | Missing, corrupt, future, and incompatible state remain distinguishable                 |
| Privacy             | Renderer, logs, evidence, artifacts, and GitHub contain no raw secrets                  |
| Update              | Exact version transition preserves active and completed work honestly                   |
| Multi-user          | Disposable user roots never share state, auth, paths, or receipts                       |

### Test Layers

1. Contract tests for Work identity and revision, task graphs, snapshots,
   deltas, workers, workspaces, actions, approvals, artifacts, receipts,
   connectors, browser targets, and schema negotiation.
2. State-machine tests for replay, snapshot/delta resync, cancellation,
   restart, expired leases, stale Work revisions, concurrent writers, partial
   corruption, future schemas, and uncertain effects.
3. Adapter contract suites using fake provider CLIs, models, Git remotes,
   GitHub, productivity services, Chromium/CDP, terminal adapters, and secure
   storage.
4. Desktop tests for fresh install, onboarding, projection agreement, panes,
   diff review, bounded incremental rendering, browser ownership, approval
   invalidation, degraded states, keyboard use, and accessibility.
5. Packaged Tauri acceptance against disposable state and a temporary checkout
   runtime port. Browser preview is not packaged-app proof.
6. Opt-in account-backed smoke tests against authorized provider, GitHub, and
   productivity test accounts before public connector certification.
7. Adversarial tests for secret and prompt injection, symlink escape, malicious
   repositories, hostile pages, forged workers, replayed approvals, renderer
   compromise, uncertain network completion, and hostile schema data.

All automated tests use disposable configuration and evidence roots. They do
not depend on maintainer accounts, warm state, existing panes, or installed
runtime data.

### Acceptance Gates and SHA-Bound Stamps

The existing layer gates remain additive prerequisites:

- `scripts/check_l0_acceptance.sh`;
- `scripts/check_l1_acceptance.sh`;
- `scripts/check_l2_acceptance.sh`;
- `scripts/hooks/stop_l0l1_gate.sh` and its exact-HEAD stamps.

L3 has accepted live evidence but no dedicated acceptance script or Stop-hook
stamp. Before the remaining connector ledger can claim full L3 completion, the
implementation must add `scripts/check_l3_acceptance.sh`, an exact-HEAD stamp,
and the same stop-gate behavior. The gate preserves the already accepted Apple
Calendar lane and adds only the connector breadth claimed complete at that
HEAD.

Each Work Fabric checkpoint receives a focused acceptance script and ledger
section before it can be marked complete:

- `scripts/check_work_fabric_a1_acceptance.sh` — durable Work plus
  one-repository loop;
- `scripts/check_work_fabric_a2_acceptance.sh` — multi-repository task graph;
- `scripts/check_work_fabric_b_acceptance.sh` — GitHub collaboration;
- `scripts/check_work_fabric_c_acceptance.sh` — productivity fusion;
- `scripts/check_work_fabric_d_acceptance.sh` — browser work surface;
- `scripts/check_work_fabric_e_acceptance.sh` — full tandem loop.

Every script writes a stamp only for a clean exact HEAD. The Stop hook blocks a
completion claim when the matching ledger section is complete but its stamp is
missing or stale. The implementation may generalize the existing hook while
preserving `scripts/hooks/stop_l0l1_gate.sh` as a compatibility entrypoint.
`check_ci_local.sh` must invoke all gates whose ledger sections claim completion
before those features can be promoted. Framework extraction and extension gates
are added only when Releases F and G are authorized.

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

Release labels are acceptance checkpoints, not a forced serial calendar. Their
dependency graph is:

```text
A1 -> A2 -> B -----------\
  \-> C ------------------+-> E -> F -> G
  \-> D -----------------/
```

Current L3 connector implementation may continue in parallel, but no new L3/L4
outcome is accepted complete until A1 produces canonical `work_id`. A2/B and D
then proceed on their own dependencies. Public alpha and framework extraction
still wait for the integrated E gate.

### Release A1 — Durable Work and One-Repository Loop

Fresh-install truth; durable `Work` production; `work_id` migration;
WorkSession snapshot/delta delivery; one local repository; one isolated
worktree; one provider-owned worker and terminal pane; Home/Work/Agent
agreement; diff, test, artifact, approval, receipt, and restart recovery.

This is independently useful: a user can complete governed local repository
work without GitHub or productivity accounts.

### Release A2 — Multi-Repository Coordination

Multi-root Workspace, `WorkTaskGraphV1`, explicit repository/path scopes,
topological scheduling, overlapping-write refusal, parallel independent lanes,
barriers, revisioned replanning, cross-repository verification, and partial
publication-saga evidence.

This is independently useful for coordinated frontend/backend/docs or monorepo
plus dependency work.

### Release B — GitHub Collaboration

Repository-scoped GitHub connection, remote divergence, branches, pull
requests, reviews, checks, review-each-publication, bounded Heiwa-branch sync,
and branch/pull-request/check receipts.

This provides an Origin-like collaboration surface while GitHub stays
canonical.

### Release C — Productivity Fusion

Extend the existing Mac-first Apple Calendar implementation; do not rebuild it.
Complete first-party Calendar, Mail, and Files contracts with connect, health,
bounded read, context reference, staged action, Action Gate execution, receipt,
revoke, and honest failure behavior. Finish Google Calendar only after its
external OAuth dependency is supplied; add `gmail.send` through the existing
approval executor; certify each additional ecosystem against the same contract.

Each connector ships only with a real read-to-action outcome.

### Release D — Browser Work Surface

Absorb roadmap L4: runtime-owned `heiwa_browser`, packaged Chromium sidecar,
dedicated profile, CDP target registry, node-aware tab ownership, explicit
handoff, bounded frame transport, typed browser actions, hostile-page handling,
Action Gate enforcement, receipts, crash recovery, and cross-platform packaged
acceptance.

This is independently useful for research, authenticated web work, previews,
and governed browser automation within a Work Session.

### Release E — Tandem Operating Loop

Certify the integrated acceptance workflow from productivity signal through
multi-repository/provider execution, browser context or action, exact approval,
GitHub result, productivity follow-up, restart, and receipt.

This is the public-alpha threshold for the broader work operating system.

### Release F — Internal Framework Extraction

After Releases A1 through E prove identical contracts across at least three
first-party surfaces, extract stable `SurfaceDefinition`, intent, event,
permission, empty-state, compatibility, and test-harness APIs. Remove
first-party special cases. Keep extension loading closed.

### Release G — Third-Party Extension Surface

Only after the internal contracts survive real use and one compatibility
migration, add signed packages, capability manifests, sandboxed execution,
explicit permissions, revocation, upgrades, and connector/surface SDKs.

No generic framework work precedes the integrated product.

## Review Resolution Record

| ID     | Review defect                                                                   | Resolution                                                                                        |
| ------ | ------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------- |
| WF-R1  | `Work` and Work Session competed as primary containers                          | `Work` is durable and keyed by `work_id`; Work Session is its read projection                     |
| WF-R2  | Operator thread ID would have become accidental Work identity                   | Threads attach to Work; V1 has a primary thread and future related threads                        |
| WF-R3  | Existing `MeshEnvelope.work_id` had no general producer                         | Release A1 creates Work before scoped domain activity and defines append-only migration           |
| WF-R4  | Fresh local Work could not supply cryptographic `origin_node` before enrollment | Local Work is installation-bound and non-replicable until signed node binding                     |
| WF-R5  | Relationship to roadmap L4 and mesh/L3 specs was unstated                       | Explicit supersedes/preserves rules and a browser Release D were added                            |
| WF-R6  | Release C read like a greenfield connector implementation                       | Accepted Apple Calendar capability is named and preserved                                         |
| WF-R7  | Release A bundled too many independently valuable outcomes                      | A1 one-repository Work loop and A2 multi-repository coordination are separate gates               |
| WF-R8  | Snapshot had no scalable incremental-delivery contract                          | Bounded snapshot, typed deltas, watermarks, pagination, backpressure, and resync are specified    |
| WF-R9  | Cross-repository dependency ordering had no mechanism                           | Revisioned DAG, scope-conflict checks, barriers, topological scheduling, and sagas are specified  |
| WF-R10 | Acceptance matrix was not connected to repository enforcement                   | Additive scripts, exact-HEAD stamps, ledgers, Stop-hook compatibility, and CI wiring are required |
| WF-R11 | Browser target lifetime risked being coupled to Work lifetime                   | Targets remain node-owned resources; only Work-scoped control events require `work_id`            |
| WF-R12 | Durable Work revision and renderer revision were conflated                      | `work_revision`, `projection_revision`, operator cursor, and source watermarks are distinct       |

## Definition of Done

This architecture program is complete only when:

- the integrated acceptance workflow passes from a clean install;
- durable `Work` is the only coordination aggregate, `work_id` is produced
  before scoped domain activity, and threads remain attached interaction
  resources;
- bounded snapshot/delta delivery and `WorkTaskGraphV1` pass recovery,
  backpressure, conflict, and stale-revision tests;
- all acceptance-matrix proofs exist at one exact protected-main commit;
- a certified release from that commit installs and updates on supported
  platforms;
- no test or user flow depends on maintainer state;
- no surface or adapter bypasses the operator stream, DREX, Action Gate,
  evidence, or secure-storage boundaries;
- GitHub source sync and private evidence remain separated;
- the roadmap's L4 browser capability passes node-aware ownership, hostile-page,
  Action Gate, crash-recovery, and packaged cross-platform acceptance;
- worker and approval legitimacy survive crash, replay, drift, and adversarial
  testing;
- existing L0-L3 evidence remains accepted and Work Fabric gates/stamps are
  additive, current, and wired into the completion hook;
- third-party extension APIs remain closed until the first-party extraction
  gate is satisfied;
- the release receipt connects decision, implementation, tests, installed
  runtime, exact commit, release artifact, and public install authority.
