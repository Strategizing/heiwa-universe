# Heiwa Persistent-World Product Map

Status: Product design reference; not a protocol contract\
Date: 2026-08-27\
Working metaphor: Fortnite-like persistence, experiences, squads, progression,
and creator energy applied to valuable digital work\
Protocol dependency: [`../superpowers/specs/2026-08-27-heiwa-work-continuity-triple-design.md`](../superpowers/specs/2026-08-27-heiwa-work-continuity-triple-design.md)

## Product Thesis

Heiwa should feel like one persistent world for AI, productivity, coding,
research, life administration, and micro-SaaS creation—not a folder of agent
chats or a dashboard of integrations.

The Fortnite comparison is a product-design tool, not literal branding, visual
copying, engagement manipulation, or protocol language. The useful mechanics
are:

- one persistent identity and world;
- many focused experiences entered without rebuilding context;
- squads of humans and agents with visible roles;
- loadouts assembled from capabilities rather than weapons;
- progression based on durable accomplishments;
- replays that prove what happened;
- creator tooling that turns successful procedures into reusable experiences;
- live evolution without invalidating the user's history.

Heiwa's version must optimize for verified outcomes, agency, calm, and true
value. It must not optimize for compulsive time-in-app, artificial streaks,
fear of missing out, loot-box economics, or permission expansion.

## The World Model

The persistent world is a product projection over durable Heiwa runtime truth.

| Product world       | Heiwa meaning                                                                                       | Current or target authority                               |
| ------------------- | --------------------------------------------------------------------------------------------------- | --------------------------------------------------------- |
| Player              | User, team member, or explicitly represented collaborator                                           | Per-user identity and future organization policy          |
| Home / Lobby        | One calm view of active Work, signals, approvals, outcomes, and resumable paths                     | Heiwa.app over bounded local read models                  |
| Mission             | One durable user objective                                                                          | `Work` / `work_id`                                        |
| Island / Experience | A focused outcome environment such as Review, Research, Build, or Launch                            | First-party surface and workflow over Work                |
| Party / Squad       | Humans, provider agents, local workers, verifiers, and assistants attached to Work                  | Worker/session identities and leases                      |
| Loadout             | Accounts, tools, models, devices, policies, budgets, and data sources available for one mission     | Capability metadata plus internal authority enforcement   |
| Match state         | Current plan, progress, blockers, pending decisions, and verified outcomes                          | Work session projection and Continuation                  |
| Boss gate           | Approval, reconciliation, spend, publication, destructive action, or human judgment boundary        | Action Gate and current local policy                      |
| Loot                | Useful artifacts: code, documents, research, automations, products, decisions, and saved procedures | Artifact and evidence references                          |
| Replay              | Inspectable timeline of decisions, actions, effects, failures, verification, and recovery           | Effect Receipts, Work events, artifacts, and source refs  |
| Locker              | User-controlled capability collection with health, privacy, cost, and scope                         | Provider/account/device registry; not ambient permission  |
| Creative            | Environment for turning a proven procedure into a reusable first-party Experience                   | Future Experience Studio after authority contracts mature |
| Discover            | Curated, trusted ways to solve goals across first-party and later third-party Experiences           | Future catalog backed by claims and conformance evidence  |
| Season              | A coherent product chapter that adds valuable world capabilities and migrations                     | Release program, never a forced engagement cycle          |

The same Work must remain recognizable in every surface. Home, Work, Agent,
browser, mobile, messaging, and external AI hosts are cameras and controls over
one world state; none becomes a competing source of truth.

## Core Player Loop

```text
interest or signal
    -> mission seed
    -> choose or recommend an Experience
    -> inspect the proposed loadout and boundaries
    -> assemble a squad
    -> execute, steer, and verify
    -> cross explicit boss gates
    -> receive useful artifacts and Effect Receipts
    -> replay or continue anywhere
    -> save the proven procedure as reusable knowledge
```

The user may enter at any point. A GitHub review, calendar event, browser tab,
message, file, idea, error, or spoken request can create or attach to Work. The
product should show the smallest next meaningful action rather than forcing the
user through a universal wizard.

## Primary Surfaces

### Home

Home is a lobby, not an analytics dashboard. It answers:

- What changed?
- What matters now?
- What is already moving safely?
- What needs me?
- What finished, and where is the proof?
- Which mission can I resume?

Home groups by Work and outcome, not by provider or connector. A user should
not need to visit separate GitHub, model, mail, calendar, or browser dashboards
to understand one mission.

### World Map

The World Map shows active and available Experiences as connected outcome
paths. It is not a map of installed software.

Examples:

- `Idea -> Validate -> Build -> Launch -> Operate`
- `Signal -> Research -> Decide -> Execute -> Verify`
- `Issue -> Reproduce -> Fix -> Review -> Release`
- `Message -> Clarify -> Schedule -> Prepare -> Follow up`

Known missing transitions appear as fog, not fake capability. The Capability
Observatory and negative-space graph can recommend where Heiwa should build or
connect the next path.

### Workroom

The Workroom is the mission-specific shared surface. It combines:

- objective and current revision;
- plan and verified progress;
- squad roles and live/stale/closed state;
- artifacts and diffs;
- approvals and reconciliation;
- Effect Receipts and evidence;
- continuation posture;
- the user's conversation and corrections.

It does not recompute truth independently from Home or Agent views.

### Squad

Squad makes delegation legible. Every member shows:

- identity and owner/provider;
- assigned role and Work scope;
- current task or waiting state;
- tools, device, workspace, budget, and authority actually held;
- last evidence and liveness;
- how to stop, replace, or hand off the member.

Agents do not become fictional personalities that obscure accountability. A
friendly presentation may exist, but provider, model, process, and authority
truth stay inspectable.

### Locker

Locker is the user's capability collection:

- connected accounts;
- provider seats and APIs;
- local models;
- devices and execution environments;
- tools and app actions;
- data sources;
- policies, budgets, and reusable procedures.

Each item shows health, freshness, locality, cost, privacy, supported actions,
and current scope. Connected does not mean executable. Installed does not mean
trusted. Previously approved does not mean currently authorized.

### Replay

Replay is a first-class evidence experience, not a log viewer. It reconstructs:

- the user decision or signal that created the mission;
- plan revisions and important corrections;
- who or what acted;
- proposed and actual effects;
- verification and uncertainty;
- artifacts and source references;
- failures, restarts, handoffs, and compensation;
- the exact outcome and what remains unresolved.

Replay can produce a shareable redacted story without exposing raw local
journals, secrets, private prompts, or unrelated Work.

### Creative

Creative turns a proven workflow into a reusable Experience definition. It
starts closed and first-party.

The author selects successful Work, removes private values, names required
capabilities, defines user-visible stages, adds fixtures, specifies failure and
approval behavior, and tests migrations. Heiwa generates no publishable package
until the procedure passes conformance and supply-chain gates.

Creative cannot safely become a third-party ecosystem until uniform Lease,
sandbox, permission, revocation, upgrade, and package-signing contracts exist.

## First-Party Experiences

### Review Arena

Greptile-like value without requiring a separate review product:

- understand repository and Work context;
- inspect a diff or branch;
- identify correctness, security, architecture, and evidence gaps;
- propose focused fixes;
- verify the exact updated state;
- produce a review replay and receipts.

GitHub remains canonical for pull requests and review publication. Review Arena
remains useful for purely local Work with no GitHub account.

### Build Foundry

Fast local or remote build execution without exposing CI infrastructure as the
user's normal workflow:

- select an eligible local or remote execution environment;
- prepare an isolated workspace;
- execute builds and tests;
- reuse safe caches;
- stream bounded logs and artifacts;
- diagnose failures;
- verify outputs and record receipts.

GitHub Actions can remain a publication adapter. It is not the user-facing
mission model, and no external build-accelerator vendor dependency or identity
is required.

### Micro-SaaS Expedition

An end-to-end path from curiosity to operating value:

1. capture a problem or audience signal;
2. gather source-backed market and user evidence;
3. define the smallest valuable product and acceptance tests;
4. build through governed code Work;
5. create brand, docs, onboarding, pricing, and launch artifacts;
6. stage domain, deployment, payment, and customer-facing effects;
7. verify the live product;
8. collect support, usage, revenue, cost, and improvement signals;
9. turn recurring operations into reusable procedures.

The Experience does not pretend all steps can be autonomous. Identity,
payments, legal promises, publication, production mutation, and customer
communication remain explicit gates.

### Research Expedition

- start from a question or observed signal;
- acquire authoritative source packs;
- preserve provenance and freshness;
- compare competing explanations;
- run bounded calculations or experiments;
- produce a decision artifact;
- keep missing evidence and uncertainty visible;
- continue later without rereading the entire corpus.

### Life and Admin Hub

- normalize calendar, mail, files, reminders, messages, and commitments into
  bounded signals;
- protect bystander and shared-account privacy;
- suggest and stage actions;
- execute only within current consent;
- return outcomes to the native software and Heiwa Replay.

Native Apple, Google, Microsoft, and other applications remain valuable. Heiwa
adds continuity across them instead of cloning every interface.

## Progression

Progression reflects durable capability and outcomes:

- a connector becomes product-grade;
- a procedure passes its evals repeatedly;
- the user saves measurable time or cost;
- an uncertain effect is reconciled;
- evidence coverage improves;
- a mission becomes safely resumable;
- a manual workflow becomes a reviewable automation;
- a first-party Experience survives a migration;
- a creator package earns a stronger trust profile.

There are no artificial daily streaks, random rewards, pay-to-win capability,
or disappearing access designed to force engagement.

## Discovery and Fog of War

Unknown unknowns are represented as discoverable product state:

- **Capability Observatory:** watches official specifications, SDKs,
  repositories, and provider changes as source packs.
- **Footprint Mapper:** with consent, inventories available local and connected
  capability metadata without inspecting content by default.
- **Negative-Space Graph:** records repeated manual fallbacks, blocked goals,
  abandoned paths, corrections, unavailable actions, and missing transitions.
- **Simulation Lab:** tests prospective paths against fixtures, hostile input,
  outages, permission shrinkage, duplicates, schema drift, and restart.
- **Executable Claim Registry:** prevents a discovered or documented capability
  from appearing as available after its evidence becomes stale.

Discovery may reveal a new world path. It cannot activate the path, grant
authority, or silently install an Experience.

## Experience Lifecycle

First-party Experiences move through:

```text
observed need
  -> product hypothesis
  -> manual Work path
  -> bounded first-party surface
  -> repeatable fixtures and receipts
  -> internal Experience definition
  -> compatibility migration
  -> signed public candidate
```

Third-party publishing remains closed until the Work Continuity design's
authority and package prerequisites are satisfied.

## Value Metrics

Heiwa measures:

- verified outcomes completed;
- elapsed user attention saved;
- cost and tokens saved without quality loss;
- risk reduced or caught before effect;
- percentage of external effects with verification;
- uncertain effects reconciled;
- successful restart and cross-surface continuation;
- capabilities moved from observed to product-grade;
- procedures reused successfully without permission widening;
- user corrections converted into durable improvements.

Time in app, message volume, agent count, and raw tool-call volume are not
success metrics.

## Failure Posture

The world stays coherent when things fail:

- A disconnected provider becomes an unavailable squad member, not a vanished
  mission.
- A stale capability disappears from eligible loadouts but remains in history.
- An uncertain external effect becomes a reconciliation gate, not an automatic
  retry.
- A schema Heiwa cannot understand becomes incompatible state, not guessed
  state.
- A revoked permission narrows the mission immediately.
- A crashed runtime rebuilds surfaces from evidence and marks process-owned
  activity stale or interrupted.
- A failed Experience cannot rewrite Work or receipts to make itself look
  successful.

## Product Sequence

1. Finish Work Fabric A1 tri-surface agreement and restart recovery.
2. Establish the Executable Claim Registry and split Call Receipt from Effect
   Receipt.
3. Make Home, Workroom, Squad, Locker, and Replay projections over the same
   bounded runtime truth.
4. Ship Review Arena and Build Foundry as first-party code Experiences.
5. Prove productivity and research Experiences against the same continuity
   triple.
6. Add Creative for first-party procedure extraction and migration testing.
7. Open public Experience packaging only after uniform authority and
   supply-chain gates exist.

## Guardrails

- The metaphor never changes runtime authority.
- No surface invents progress or capability to make the world feel populated.
- Providers retain their own auth, prompts, sessions, quotas, and native UX.
- Existing software remains the canonical home for its domain data when
  appropriate.
- Every risky effect is staged, authorized, verified, and receipt-backed.
- Local truth and user control survive loss of any cloud provider.
- Product delight comes from continuity and leverage, not hidden access.
