# Heiwa Work Continuity Triple

Status: Draft for review\
Date: 2026-08-27\
Decision class: Architecture and future public interoperability\
Planes: Intake, Execution, Evidence\
Depends on: Work Fabric Release A1\
Product companion: [`../../design/2026-08-27-heiwa-persistent-world-product-map.md`](../../design/2026-08-27-heiwa-persistent-world-product-map.md)

## Decision

Heiwa will develop a transport-neutral Work Continuity Envelope, but it will not
publish the full Work/Capability/Lease/Proposal/Effect/Receipt/Continuation
vocabulary as version 1.

The first public candidate is the **continuity triple**:

1. **Work** — stable identity and revisioned state for the user's objective.
2. **Effect Receipt** — evidence of an attempted or confirmed side effect.
3. **Continuation** — a bounded, restart-safe view of what another authorized
   surface needs to continue, reconcile, or stop the Work.

The target public namespace is `wce/v1`. Until every publication gate in this
design passes, implementations use the experimental namespace
`wce/x-continuity` and make no compatibility promise.

Capability, Lease, and Action Proposal remain experimental under `wce/x-*`.
They cannot enter `wce/v1` until Heiwa enforces one authority model across
tools, workspaces, connectors, agents, devices, and external effects.

Experience Packages remain a first-party product concept. No third-party
Experience installation or activation contract is authorized by this design.

## Why This Is Heiwa's Layer

MCP connects models and agents to tools and resources. A2A exchanges agent
tasks, messages, and artifacts. WebMCP lets web applications expose structured
tools. Apple App Intents and Android AppFunctions expose application actions to
operating-system intelligence. Those systems are transports and capability
surfaces that Heiwa should consume and serve; Heiwa does not replace them.

The continuity triple answers a different set of questions:

- Which durable user objective caused this activity?
- Which revision of that objective was in force?
- Did an external effect happen, fail, remain uncertain, or get denied?
- What evidence and external identifiers support that answer?
- Can an authorized surface resume safely without repeating the effect?
- When must execution stop for reconciliation, approval, or human input?

Heiwa's distinctive boundary is therefore continuity and proof across existing
software, providers, devices, and agent transports.

## Current Repository Truth

This design starts from implementation, not desired vocabulary.

| Candidate contract | Truth at `dev` commit `52dca559`                                                                                                                  | Decision                                                                                                   |
| ------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------- |
| Work               | `heiwa_work` has a stable `WorkId`, aggregate, events, migration, projector, snapshot, and delta mechanics.                                       | Use as the reference aggregate. Publish only a bounded projection, not the raw journal or internal struct. |
| Continuation       | Snapshot/delta delivery, cursor recovery, interrupted-turn recovery, and lease revocation exist. No canonical type is named `Continuation`.       | Introduce a derived continuation view only after its no-repeat-effect invariants are tested.               |
| Receipt            | `heiwa_receipts::Receipt` records cost-bearing calls. `ToolCallReceipt`, `RunReceipt`, and `PersistedRunReceipt` describe other partial outcomes. | Split call accounting from external-effect proof before any public schema.                                 |
| Lease              | `WriterLease` is durable, exclusive, expiring, and crash-revoked. `ToolLease` is an allowlist row. Other lease shapes differ again.               | Do not publish a generic Lease contract. Preserve each honest scope until an enforced common model exists. |
| Capability         | Advertisements, flags, tool schemas, and capability strings exist, but no uniform risk/privacy/locality/cost/reversibility contract exists.       | Keep experimental and adapt existing protocols rather than freezing a Heiwa vocabulary.                    |
| Action Proposal    | Approval and action-gate mechanics exist, but no canonical proposal object exists.                                                                | Keep experimental until one object is enforced from staging through effect receipt.                        |

The current L0-L2 acceptance ledgers and exact-commit stamps are a small working
precedent for executable claims: a declaration is not accepted without a named
verifier and evidence tied to source state. They are not themselves the future
public claim registry.

## Semantic Boundary

The continuity triple is a semantic overlay, not a new network transport.

```text
provider or app transport
        |
        v
bounded Work projection
        |
        v
attempted external effect -> Effect Receipt
        |
        v
Continuation view -> continue | reconcile | request input | stop
```

An adapter may carry the triple through MCP metadata, A2A task metadata and
artifacts, a local API, an operating-system intent bridge, or a future
protocol. The meaning of Work, Effect Receipt, and Continuation must not change
with the transport.

No adapter may treat successful message delivery, process exit, or model text
as proof that an external effect occurred.

## Contract 1: Work

### Authority

The local Work aggregate remains canonical on its owning Heiwa installation.
The public shape is a bounded projection and never grants mutation authority.

### Required projection fields

`WorkProjectionV1` contains:

| Field        | Meaning                                                                                                                 |
| ------------ | ----------------------------------------------------------------------------------------------------------------------- |
| `schema`     | Exact schema identifier, initially `wce/x-continuity/work@1`.                                                           |
| `work_id`    | Stable Work identity; never an operator thread, task, run, or provider-session identifier.                              |
| `revision`   | Monotonic accepted Work revision.                                                                                       |
| `status`     | `active`, `blocked`, `cancelled`, `failed`, or `complete`.                                                              |
| `objective`  | Optional bounded and redacted user-facing objective. Omission must not prevent identity or reconciliation.              |
| `origin_ref` | Opaque installation or node reference appropriate to the recipient; it must not expose a stable global user identifier. |
| `created_at` | Source timestamp for Work creation.                                                                                     |
| `updated_at` | Source timestamp of the latest accepted Work event.                                                                     |
| `projection` | Epoch, cursor, omitted counts, redaction applied, and source evidence references.                                       |

### Invariants

- `work_id` is created or adopted before Work-scoped activity.
- A thread, provider session, task, pane, repository, or device may attach to
  Work but cannot replace its identity.
- A stale writer cannot overwrite a later revision.
- A projection never exports raw journal lines, secrets, unbounded messages, or
  unrelated Work.
- Missing or conflicting historical Work identities are explicit migration
  conflicts, never silently merged.
- Local-only Work does not cross a replication or guest boundary until an
  authorized origin reference exists.

## Contract 2: Call Receipt and Effect Receipt

### Required semantic split

Before `wce/v1`, the current receipt noun is split conceptually and in code:

- **Call Receipt** records model/tool-call economics and execution telemetry:
  provider, model, tokens, latency, attempts, and cost.
- **Effect Receipt** records the truth of an externally observable or durable
  side effect: file mutation, branch publication, message send, calendar
  change, payment, booking, account change, or equivalent action.

A Call Receipt may reference an Effect Receipt, but neither substitutes for the
other. A successful model call can produce no effect. An effect can occur even
when the caller loses the response and reports an error.

Legacy `Receipt`, `RunReceipt`, `PersistedRunReceipt`, and `ToolCallReceipt`
remain internal compatibility types until an implementation plan defines their
migration. The public candidate does not alias any of them.

### Required Effect Receipt fields

`EffectReceiptV1` contains:

| Field                     | Meaning                                                                                           |
| ------------------------- | ------------------------------------------------------------------------------------------------- |
| `schema`                  | Exact schema identifier, initially `wce/x-continuity/effect-receipt@1`.                           |
| `effect_receipt_id`       | Stable receipt identity.                                                                          |
| `work_id`                 | Durable Work that caused the attempt.                                                             |
| `work_revision`           | Work revision admitted for the attempt.                                                           |
| `effect_kind`             | Stable action category, namespaced by adapter or standard.                                        |
| `target_ref`              | Bounded target identifier safe for the recipient.                                                 |
| `idempotency_key`         | Optional stable deduplication key when the external system supports or Heiwa can enforce one.     |
| `status`                  | `confirmed`, `failed`, `denied`, `uncertain`, or `compensated`.                                   |
| `actor_ref`               | Human, agent, runtime, or service identity that executed the attempt.                             |
| `adapter_ref`             | Exact adapter and version used.                                                                   |
| `started_at` / `ended_at` | Attempt interval.                                                                                 |
| `request_digest`          | Digest of the admitted action payload after canonicalization; never a raw secret-bearing payload. |
| `external_refs`           | Bounded provider identifiers such as commit, message, event, transaction, or job IDs.             |
| `evidence_refs`           | Local evidence, artifact, diff, log, screenshot, or verifier references.                          |
| `verification`            | Verification method, result, timestamp, and verifier identity.                                    |
| `compensation`            | `not_available`, `available`, `attempted`, `confirmed`, or `failed`, plus bounded references.     |
| `redaction`               | Fields omitted or transformed for this recipient.                                                 |

### Effect status rules

- `confirmed` requires external read-back, deterministic local verification, or
  another verifier appropriate to the effect. A zero exit code alone is not
  sufficient for a remote mutation.
- `failed` means the effect is proven not to have occurred. An absent response
  is not proof of failure.
- `denied` means policy refused execution before the effect boundary.
- `uncertain` means Heiwa cannot prove whether the effect occurred. It blocks
  automatic repetition until reconciliation.
- `compensated` means the original effect occurred and a later compensating
  action was confirmed. It does not rewrite history to `failed`.

## Contract 3: Continuation

### Purpose

Continuation is a derived, bounded decision view. It is not serialized process
memory, a provider transcript, or permission to repeat unfinished calls.

`ContinuationViewV1` contains:

| Field               | Meaning                                                                                                             |
| ------------------- | ------------------------------------------------------------------------------------------------------------------- |
| `schema`            | Exact schema identifier, initially `wce/x-continuity/continuation@1`.                                               |
| `work_id`           | Work being continued or reconciled.                                                                                 |
| `work_revision`     | Revision on which the view was built.                                                                               |
| `projection_epoch`  | Identity of the current fold; prevents deltas from different rebuilds being merged.                                 |
| `cursor`            | Durable source position for replay or later delta requests.                                                         |
| `work_status`       | Current durable Work status.                                                                                        |
| `resumability`      | `resumable`, `input_required`, `approval_required`, `reconcile_required`, `blocked`, `terminal`, or `incompatible`. |
| `pending_inputs`    | Bounded questions or missing material, without hidden model chain-of-thought.                                       |
| `pending_approvals` | Approval references and expiry; never approval authority itself.                                                    |
| `uncertain_effects` | Effect Receipt references that require reconciliation before retry.                                                 |
| `latest_effects`    | Bounded terminal Effect Receipt references relevant to the next decision.                                           |
| `artifact_refs`     | Bounded outputs required to understand or continue the Work.                                                        |
| `checkpointed_at`   | Time the view was derived.                                                                                          |
| `redaction`         | Omitted counts, reasons, and recipient policy.                                                                      |

### Resume invariants

- A Continuation view never widens authority. The receiving surface must obtain
  current local authorization through its native policy mechanism.
- An expired or revoked approval, lease, account grant, or provider session is
  re-evaluated; historical validity does not continue automatically.
- `uncertain` effects force `reconcile_required`.
- Non-idempotent effects are never repeated solely because a process or network
  call lacks a terminal response.
- Unknown future schema versions are preserved as opaque evidence where safe,
  counted, and surfaced as `incompatible`; they are not guessed.
- A terminal Work cannot return to active through Continuation alone.
- Provider-native session restoration may improve context, but it never becomes
  the Work system of record.

## Executable Claim Registry

The registry is the first implementation dependency because it makes every
standard claim falsifiable.

### Claim declaration

Each tracked claim declares:

| Field             | Meaning                                                                                                 |
| ----------------- | ------------------------------------------------------------------------------------------------------- |
| `claim_id`        | Stable repository-wide identifier.                                                                      |
| `subject`         | Schema, behavior, adapter, surface, or compatibility profile being claimed.                             |
| `claim`           | One precise, testable statement. Compound marketing claims are refused.                                 |
| `required_state`  | Minimum computed state required by the consumer, normally `implemented` or `verified`.                  |
| `scope`           | Files, crates, schemas, adapters, and environments whose change can invalidate the claim.               |
| `verifier_id`     | Repository-owned verifier selected from an allowlist; manifests cannot inject arbitrary shell commands. |
| `evidence_policy` | Required tests, receipts, external probes, redaction, and freshness.                                    |
| `compatibility`   | Schema versions and profiles covered by the verifier.                                                   |
| `expiry`          | Time- or source-change condition after which the claim downgrades.                                      |

### Computed truth

The registry computes `planned`, `implemented`, `verified`, `degraded`, or
`retired`; manifests do not declare their own observed state. `verified` is
never trusted from prose. Verification evidence binds the claim to:

- exact source commit or content digest;
- verifier version and result;
- bounded platform/environment profile;
- timestamp and freshness policy;
- relevant receipts or external identifiers;
- redaction applied.

When scope, dependency versions, external behavior, or freshness changes, the
claim becomes `degraded` until reverified. Retired code or schemas force
`retired`; documentation cannot keep them current.

The registry must detect at least:

- named symbols or files that no longer exist;
- canonical docs that conflict with current architecture truth;
- schemas without serialization and migration tests;
- public claims whose required guest adapter is absent;
- stale external-spec or provider assumptions;
- verification evidence tied to an older incompatible source state.

### Existing precedent and required replacement

The L0-L2 ledger/stamp/hook pattern demonstrates claim, verifier, and
source-bound evidence. The provider-specific `.claude/*-accept-sha` files and
exact-HEAD logic remain local acceptance mechanics. The general registry must
be provider-neutral, scope-aware, machine-readable, and usable by local gates,
CI, release certification, and product surfaces.

## Adapter Profiles

### Host profile

Heiwa consumes another system's tools, agents, intents, or application
functions while retaining local Work and receipt authority.

Host consumption is necessary but is not sufficient evidence for a public
standard because Heiwa controls both the mapping and the result interpretation.

### Guest profile

Heiwa exposes bounded Work, Effect Receipt, and Continuation views to another
host through an existing transport.

The guest profile is the publication gate. A conforming external host must be
able to:

1. receive a bounded Work projection;
2. observe or request Effect Receipts without raw journal access;
3. request a Continuation view after disconnect or restart;
4. detect `reconcile_required`, `incompatible`, and terminal states;
5. preserve `work_id`, revision, receipt IDs, and cursor without inventing
   replacements;
6. avoid gaining mutation authority from the continuity payload.

### Initial transport mappings

- **A2A:** map `work_id` and revision into task metadata; carry bounded Effect
  Receipt and Continuation payloads as typed data or artifacts. A2A task state
  remains transport state, not canonical Work state.
- **MCP:** expose read-only Work, Effect Receipt, and Continuation resources or
  tools. Mutation tools remain governed by Heiwa's internal experimental
  authority layer.
- **Local API:** provide the reference projection and replay behavior for
  Heiwa.app and conformance tests.
- **OS and Web intents:** adapters create or attach Work and record Effect
  Receipts around the native call. The operating-system or page declaration
  does not become Heiwa authority.

## Failure and Recovery Semantics

| Condition                                        | Required result                                                                   |
| ------------------------------------------------ | --------------------------------------------------------------------------------- |
| Unknown Work                                     | Refuse continuation; never mint a replacement implicitly.                         |
| Stale Work revision                              | Return conflict and current bounded revision/cursor.                              |
| Cursor from another projection epoch             | Return resync required.                                                           |
| Unknown schema version                           | Return incompatible while retaining bounded opaque evidence where safe.           |
| External timeout after dispatch                  | Emit `uncertain` Effect Receipt and require reconciliation.                       |
| Verification disagrees with execution response   | External read-back wins; preserve both observations in evidence.                  |
| Approval or authority expired                    | Return approval required or blocked; do not inherit past authority.               |
| Adapter unavailable                              | Preserve Work and continuation state locally; surface honest retry posture.       |
| Restart with open process-owned activity         | Close or mark stale through durable recovery events; do not fabricate completion. |
| Redaction removes required continuation material | Return blocked with the missing-material class rather than weakening redaction.   |

## Security and Privacy

- The triple carries continuity, not ambient authority.
- Raw secrets, bearer tokens, provider transcripts, hidden reasoning, and
  unbounded user data never appear in public projections.
- Identifiers are scoped and opaque. WCE does not create a universal
  cross-service user identifier.
- Bystander or shared-resource data uses the strictest applicable ownership and
  consent policy.
- Effect Receipts record digests and bounded references instead of secret-bearing
  payloads.
- Deletion and retention policy may redact payload material while preserving a
  tombstone that an effect and later redaction occurred.
- A recipient cannot use receipt possession as permission to repeat, undo, or
  extend an action.
- Every guest adapter is threat-modeled for prompt injection, confused deputy,
  permission laundering, replay, correlation, and schema downgrade.

## Conformance Profiles

No single badge claims universal compliance. The registry reports profiles:

1. **Work Producer** — stable identity, revisions, migrations, bounded
   projection, and conflict handling.
2. **Effect Recorder** — complete status semantics, idempotency posture,
   verification, uncertainty, compensation, and redaction.
3. **Continuation Producer** — restart/replay, cursor and epoch handling,
   pending-input/approval states, and no-repeat-effect behavior.
4. **Guest Adapter** — cross-host exchange without authority widening.
5. **External Consumer** — an independently implemented consumer survives the
   compatibility corpus and one schema migration.

Every profile has deterministic fixtures for normal flow, restart, duplicate
delivery, stale revision, invalid cursor, unknown schema, revoked authority,
uncertain effect, compensation, redaction, and partial external failure.

## Public `wce/v1` Publication Gates

All gates are mandatory:

1. `CallReceipt` and `EffectReceipt` are semantically and programmatically
   separate; retired STDB wording is absent from the reference boundary.
2. The executable claim registry verifies every public contract claim and
   automatically degrades stale evidence.
3. Work, Effect Receipt, and Continuation pass the conformance corpus in three
   structurally different first-party domains: code/workspace, productivity,
   and browser or research.
4. At least one guest adapter exposes the triple to an external host.
5. At least one independently implemented external consumer passes the
   consumer profile.
6. The external consumer survives one compatibility migration without losing
   identity, repeating an uncertain effect, or silently dropping evidence.
7. Threat model, privacy model, redaction fixtures, and revocation behavior are
   published with the schema.
8. A certified installed Heiwa release proves the exact reference
   implementation and conformance evidence.

Until all eight pass, product and documentation must say
`experimental continuity profile`, never `public standard` or `wce/v1`
compatibility.

## Delivery Program

This architecture is too broad for one implementation plan. After approval it
decomposes in this order:

### Program 0 — Claim truth and receipt taxonomy

- executable claim registry;
- drift checks for canonical documents, symbols, schemas, and verifiers;
- internal `CallReceipt` naming and compatibility migration;
- retirement of STDB receipt comments and mirror vocabulary;
- Effect Receipt design fixtures, without claiming external effects yet.

### Program 1 — Effect evidence

- `EffectReceiptV1` internal experimental type;
- append/replay/projector support in `heiwa_evidence`;
- deterministic local-file and repository-effect adapters;
- uncertainty, verification, idempotency, and compensation tests;
- bounded app/CLI projections.

### Program 2 — Continuation view

- `ContinuationViewV1` derived from Work and evidence;
- restart, cursor, epoch, approval, reconciliation, and incompatibility states;
- no-repeat-effect acceptance gate;
- Home/Work/Agent agreement over the same projection.

### Program 3 — Guest interoperability

- read-only local reference API;
- one A2A or MCP guest adapter selected by strongest existing implementation;
- external consumer fixture and compatibility migration;
- public-candidate threat model and conformance kit.

### Program 4 — Authority experiments

Capability, Lease, and Action Proposal continue under `wce/x-*`. Their
implementation plans may proceed only where they close a real first-party
effect lane. No generic authority framework or third-party Experience system
precedes uniform enforcement.

## Product Relationship

Heiwa's persistent-world product can move ahead using first-party code and the
continuity triple:

- a mission is Work;
- the shared replay is Effect Receipts plus artifacts;
- moving between Home, Work, Agent, mobile, browser, or another AI host uses
  Continuation;
- squads, loadouts, approvals, and creator Experiences remain internal product
  projections until the experimental authority contracts are proven.

This keeps the product expansive without making the public standard claim more
than the runtime can enforce.

## Non-Goals

- Replacing MCP, A2A, WebMCP, App Intents, AppFunctions, OpenAPI, or native app
  APIs.
- Publishing a universal agent identity, capability, permission, lease, or
  payment standard.
- Exporting Heiwa's raw journal, provider sessions, secrets, or hidden model
  reasoning.
- Treating A2A task state, MCP call success, process exit, or model output as
  proof of an external effect.
- Launching a plugin or Experience marketplace before signed packages,
  uniform leases, sandboxing, revocation, upgrades, and supply-chain evidence
  exist.
- Freezing internal Rust structs as the public wire format.
- Using Fortnite terminology in the public infrastructure specification.

## References

- [`HEIWA.md`](../../../HEIWA.md)
- [`../../capability-fabric.md`](../../capability-fabric.md)
- [`2026-08-22-heiwa-work-fabric-design.md`](2026-08-22-heiwa-work-fabric-design.md)
- [`2026-08-20-heiwa-mesh-runtime-design.md`](2026-08-20-heiwa-mesh-runtime-design.md)
- [Model Context Protocol](https://modelcontextprotocol.io/)
- [Agent2Agent Protocol](https://a2a-protocol.org/)
- [WebMCP draft](https://webmachinelearning.github.io/webmcp/)
- [Apple App Intents](https://developer.apple.com/documentation/appintents)
- [Android AppFunctions](https://developer.android.com/ai/appfunctions)
