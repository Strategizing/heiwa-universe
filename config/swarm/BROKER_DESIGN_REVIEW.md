# heiwa-broker Extraction — Adversarial Design Review

> **Author:** Antigravity (Class 3 — Strategic Review)
> **Date:** 2026-03-06
> **Status:** ✅ APPROVED by operator 2026-03-06T13:25 PST — locked for Codex implementation
> **Canonical Reference:** `config/swarm/BUILD_BLUEPRINT_2026-03-06.md`

---

## 0. Decision Anchor: State Permissions (Question 4 — First)

Per operator instruction, this question is answered before everything else because it determines the blast radius of every subsequent decision.

### Decision: Spine retains all SpacetimeDB write ownership. Broker communicates purely over NATS.

**Rationale:**
- SpacetimeDB migration (Phase D) hasn't happened yet. Current state is SQLite/Postgres.
- Giving broker direct STDB write credentials before the schema is stable creates two state writers with potentially divergent views.
- NATS is the established communication backbone between agents — broker fits naturally as a NATS subscriber/publisher.
- When Phase D lands with a stable schema, broker can be promoted to direct STDB writes via a single config change (adding credentials) rather than a structural rearchitecture.

**Concrete Phase D Promotion Config:**

The broker reads a single env var to determine state write mode. No code changes required:

```env
# Phase B (current): broker is pure NATS, zero state writes
HEIWA_BROKER_STATE_MODE=nats_only

# Phase D (future): broker writes directly to SpacetimeDB
HEIWA_BROKER_STATE_MODE=stdb_direct
SPACETIMEDB_BROKER_URI=wss://stdb.heiwa.ltd
SPACETIMEDB_BROKER_TOKEN=<from vault>
```

Broker code gates all state writes behind `HEIWA_BROKER_STATE_MODE`. When `nats_only`, enriched envelopes are returned to Spine via NATS reply only. When `stdb_direct`, broker additionally writes task state to SpacetimeDB's `tasks` table via reducer. The NATS reply still fires in both modes for backward compatibility.

**Implication for all subsequent decisions:**
- Broker never touches the database directly (in Phase B).
- All state mutations flow through Spine → current DB (SQLite/Postgres) → future SpacetimeDB.
- Broker is pure compute: intake envelope, classify, score, route, emit enriched envelope. Zero state ownership.

> [!IMPORTANT]
> This constraint eliminates an entire class of consistency problems. It means broker failure is always recoverable without data corruption — Spine simply processes unenriched envelopes directly.

---

## 1. Minimum Viable Interface: Spine ↔ Broker

### Recommended: NATS Pub/Sub with Two New Subjects

```
Spine → heiwa.broker.route    (full IntentEnvelope)
Broker → heiwa.tasks.exec     (enriched envelope with compute_class, risk_level, assigned_worker)
Broker → heiwa.tasks.status   (lifecycle events: ANALYZED, ROUTED)
```

### What Spine Publishes to `heiwa.broker.route`

After auth validation and initial envelope normalization, Spine publishes the authenticated envelope:

```json
{
  "request_id": "req-a1b2c3d4",
  "task_id": "task-xxx",
  "raw_text": "deploy the status page to production",
  "sender_id": "devon#1234",
  "source_surface": "discord",
  "response_channel_id": 123,
  "response_thread_id": null,
  "auth_validated": true
}
```

### What Broker Publishes to `heiwa.tasks.exec`

Broker adds these fields and publishes the enriched execution envelope:

```json
{
  "request_id": "req-a1b2c3d4",
  "task_id": "task-xxx",
  "intent_class": "operate",
  "risk_level": "high",
  "compute_class": 4,
  "assigned_worker": "railway",
  "requires_approval": true,
  "steps": [...],
  "normalization": {...}
}
```

### Why Not gRPC or Direct Python Call?

| Option | Pros | Cons | Verdict |
|:---|:---|:---|:---|
| **NATS pub/sub** | Consistent with existing agent patterns, zero new dependencies, natural fallback (Spine just skips broker), broker can be on any node | Slightly higher latency than direct call (~1ms) | ✅ **Selected** |
| Direct Python call | Lowest latency, simplest to implement | Couples Spine to broker process lifecycle, can't distribute to Node B later, no fallback path | ❌ Rejected |
| gRPC | Strong typing, streaming | New dependency, overkill for this interface, adds build complexity | ❌ Rejected |

### Interface Contract

```python
# What Spine sends to heiwa.broker.route
class BrokerRouteRequest:
    request_id: str       # MUST be echoed unchanged on BrokerRouteResult
    task_id: str
    raw_text: str
    sender_id: str
    source_surface: str   # "discord" | "cli" | "api" | "web"
    response_channel_id: int | str
    response_thread_id: int | str | None
    auth_validated: bool  # always True (Spine validated)
    timestamp: float
    envelope_version: str # "2026-03-06" — fail-loud if mismatched

# What Broker replies with via NATS request-reply
class BrokerRouteResult:
    request_id: str       # MUST match the request — Spine correlates on this
    task_id: str
    intent_class: str     # from Intent Taxonomy
    risk_level: str       # low | medium | high | critical
    compute_class: int    # 1-4
    assigned_worker: str  # node hint, not binding
    requires_approval: bool
    steps: list[dict]     # planned execution steps
    normalization: dict   # intent normalizer output
    raw_text: str         # preserved original
    response_channel_id: int | str
    response_thread_id: int | str | None
    envelope_version: str # echo back — Spine validates match
```

> [!IMPORTANT]
> **Correlation invariant:** `BrokerRouteResult.request_id` MUST equal `BrokerRouteRequest.request_id`. Spine MUST reject any result where `request_id` doesn't match the pending request, especially with concurrent tasks in flight. This prevents stale or misrouted enrichments from corrupting task state.

---

## 2. Broker Unavailability — Failure Modes & Fallback

### Scenario: Broker is down or unresponsive

**Current behavior (monolithic Spine):** Spine does intent classification, planning, and dispatch inline. Everything works.

**Target behavior with broker extracted:** Spine publishes to `heiwa.broker.route`. If broker doesn't respond, Spine needs a fallback.

### Recommended: Timeout-based Fallback to Inline Processing

```
1. Spine publishes to heiwa.broker.route
2. Spine waits for response on heiwa.broker.route.result.{task_id} (request-reply pattern)
3. If response within 5s → use broker's enriched envelope
4. If timeout → Spine falls back to inline processing (current behavior)
5. Emit TASK_STATUS with "DISPATCHED_FALLBACK" to indicate broker bypass
```

**Why this works:**
- Zero behavior change when broker is down — system degrades gracefully to current behavior
- No halting — the swarm never stops processing because broker is unavailable
- Broker becomes an accelerator/enricher, not a critical gate
- Fallback is already battle-tested (it's what runs today)

> [!WARNING]
> **Anti-pattern to avoid:** Do NOT make the broker a synchronous gate that blocks all task processing. If broker becomes a hard dependency, a single broker crash halts the entire swarm. This violates the "no single point of failure" principle.

### Fallback Path Code Sketch

```python
# In Spine.handle_request(), after auth validation:
try:
    # Attempt broker enrichment (NATS request-reply, 5s timeout)
    enriched = await self.nc.request(
        "heiwa.broker.route",
        json.dumps(authenticated_envelope).encode(),
        timeout=5.0
    )
    payload = json.loads(enriched.data.decode())
    dispatch_status = "DISPATCHED_PLAN"
except TimeoutError:
    # Broker unavailable — fall back to inline planning
    logger.warning("⚠️ Broker timeout for %s. Falling back to inline planning.", task_id)
    payload = self.planner.plan(...)  # existing logic
    dispatch_status = "DISPATCHED_FALLBACK"
```

---

## 3. Extraction Order — Minimum Blast Radius

The three responsibilities to extract from Spine are:

| # | Responsibility | Current Location | Risk |
|:---|:---|:---|:---|
| 1 | Intent Classification | `spine.py` → `planner.plan()` → `IntentNormalizer.normalize()` | **Low** — pure function, no side effects |
| 2 | Risk Scoring | Not implemented yet (Phase B Action 4) | **Low** — new code, no existing callers |
| 3 | Compute Class Routing | Not implemented yet | **Medium** — determines which executor receives work |
| 4 | Task Planning (step decomposition) | `spine.py` → `planner.plan()` | **Medium** — complex, has fallback path |

### Recommended Extraction Order

```
Step 1: Risk Scorer (new code → broker)          Blast radius: ZERO
Step 2: Intent Classifier (move to broker)        Blast radius: LOW
Step 3: Compute Class Router (new code → broker)  Blast radius: LOW  
Step 4: Task Planner (move to broker)             Blast radius: MEDIUM
```

**Step 1 first** because the Risk Scorer doesn't exist yet. Writing it as a broker module from day one means it never lives in Spine — zero migration cost.

**Step 2 second** because `IntentNormalizer` is already a pure function. Moving it to broker means Spine no longer needs to import it, but the logic is identical.

**Step 3 third** because compute class routing is new logic that naturally belongs in the broker's scheduling role.

**Step 4 last** because the planner has the most complex interaction with Spine (fallback paths, step dispatch). Moving it requires verifying the entire dispatch chain still works.

### Per-Step Verification Gates

Each step gets a gate before the next proceeds. **No step advances until its specific test artifact passes.**

| Step | Component | Gate Test | Test Artifact | Acceptance Criteria |
|:---|:---|:---|:---|:---|
| 1 | Risk Scorer | Bus smoke test with risk scoring in broker | `apps/heiwa_hub/actions/smoke_test.py` | Exit 0. `DISPATCHED_PLAN` status payload contains `risk_level` field. ACKNOWLEDGED → DISPATCHED_PLAN → PASS → DELIVERED in <30s. |
| 2 | Intent Classifier | 50-request test set through broker classifier | `apps/heiwa_hub/tests/test_intent_classifier.py` reading `intent_classifier_test_set.json` | ≥95% accuracy (≥48/50 correct). Adversarial cases classified or safely fallback to `general`. |
| 3 | Compute Router | Compute class assignment validation | `apps/heiwa_hub/tests/test_compute_router.py` | All 7 intent classes produce correct compute_class (1-4). Privacy=sovereign forces class 1/2. |
| 4 | Task Planner | Full Discord smoke with broker-planned steps | `apps/heiwa_hub/actions/smoke_test_discord.py` | Exit 0. All 4 bus transitions fire. Bot posts to #smoke-test. Task ID + probe marker present. <30s. |
| **Final** | **Dual-mode** | **Smoke passes with AND without broker** | Both `smoke_test.py` and `smoke_test_discord.py` | Run once with `HEIWA_ENABLE_BROKER=true`, once with `HEIWA_ENABLE_BROKER=false`. Both exit 0. |

---

## 4. State Write Permissions — Resolved (See Section 0)

### Summary

| Entity | Reads | Writes | Justification |
|:---|:---|:---|:---|
| **Spine** | DB (SQLite/Postgres → future STDB) | DB (all state mutations) | Single state owner reduces consistency surface |
| **Broker** | Nothing persistent | Nothing persistent | Pure compute enrichment, NATS in/out only |
| **Executor** | Task instructions via NATS | Results via NATS → Spine writes | Results flow through NATS, Spine persists |
| **Messenger** | Task status via NATS | Discord API only | External delivery, no internal state writes |

### Phase D Promotion Path

When SpacetimeDB migration is complete and schema is stable:

1. Broker receives STDB credentials via environment config
2. Broker writes directly to `tasks` table via SpacetimeDB reducer
3. Spine subscribes to STDB table delta instead of NATS status
4. NATS subject `heiwa.tasks.status` becomes redundant (backward-compat retained for log consumers)

**This is a config change, not a structural change.** The broker envelope interface stays identical.

---

## 5. NATS Subject Changes

### New Subjects (Phase B)

```
heiwa.broker.route              # Spine → Broker (IntentEnvelope for enrichment)
heiwa.broker.route.result.{id}  # Broker → Spine (reply with enriched envelope)
```

### Unchanged Subjects

```
heiwa.tasks.exec                # Broker OR Spine → Executor (dispatch)
heiwa.tasks.status              # All lifecycle events (no change)
heiwa.tasks.exec.result         # Executor → merger/messenger (no change)
heiwa.core.request              # Legacy ingress (deprecated, still handled)
heiwa.tasks.new                 # V2 ingress (no change)
```

### Subject Lifecycle Plan

| Subject | Phase A | Phase B | Phase D |
|:---|:---|:---|:---|
| `heiwa.core.request` | ✅ Active | ✅ Active (compat) | ❌ Remove |
| `heiwa.tasks.new` | ✅ Active | ✅ Active | ✅ Active |
| `heiwa.broker.route` | — | ✅ New | ✅ Active |
| `heiwa.tasks.status` | ✅ Active | ✅ Active | ⚠️ STDB delta preferred |

---

## 6. Adversarial Challenges — Things That Could Go Wrong

### Challenge 1: Broker adds latency to every task

**Risk:** Adding a NATS hop adds ~1-5ms. For the current scale (single founder, <100 tasks/day), this is irrelevant.

**Mitigation:** Request-reply pattern means total added latency is 1 round-trip. The 5s timeout ensures this never blocks.

**Verdict:** Not a real problem at this scale. Monitor if task volume exceeds 1000/day.

### Challenge 2: Broker becomes a second brain that diverges from Spine

**Risk:** If broker develops its own understanding of task state (e.g., caching routing decisions), it could diverge from Spine's view.

**Mitigation:** Broker is stateless by design (Section 0). It receives an envelope, enriches it, returns it. No caching, no session memory, no local state.

**Verdict:** Mitigated by architecture. Enforce via code review — broker must not introduce any persistent state.

### Challenge 3: Envelope schema drift between Spine and Broker

**Risk:** Spine and Broker are separate processes. If one updates the envelope schema without the other, routing breaks silently.

**Mitigation:** Both import `IntentEnvelope` from `heiwa_protocol`. Schema changes are a single file. Add a version field to the envelope and fail-loud if mismatched.

```json
{"envelope_version": "2026-03-06", ...}
```

**Verdict:** Mitigated, but add envelope version validation as a Phase B gate.

### Challenge 4: Broker extraction creates two codepaths (broker-enriched vs fallback)

**Risk:** Bugs could hide in one path that isn't exercised in the other. Tests might only exercise one path.

**Mitigation:** The Discord smoke test must pass in both modes: with broker running AND with broker offline (fallback). Add both to CI.

**Verdict:** Add dual-mode smoke test to Phase B gate.

---

## 7. Implementation Sequence for Codex

After operator approval, Codex should implement in this exact order:

```
1. Create apps/heiwa_hub/agents/broker.py
   - Extends BaseAgent
   - Subscribes to heiwa.broker.route
   - Imports IntentNormalizer, RiskScorer (new), ComputeClassRouter (new)
   - Pure NATS reply: receives envelope → enriches → replies

2. Create apps/heiwa_hub/cognition/risk_scorer.py (Action 4)
   - Rule-based: intent class defaults + keyword escalators
   - No external dependencies

3. Create apps/heiwa_hub/cognition/compute_router.py
   - Maps (intent_class, risk_level) → compute_class
   - Reads ai_router.json for model registry
   - Returns compute_class (1-4) and worker hint

4. Modify apps/heiwa_hub/agents/spine.py
   - After auth validation, attempt NATS request to heiwa.broker.route
   - Timeout fallback to inline processing
   - Preserve all existing behavior as fallback path

5. Modify apps/heiwa_hub/main.py
   - Add BrokerAgent to boot sequence
   - Make it optional (like Messenger): skip if env flag says so

6. Create apps/heiwa_hub/tests/intent_classifier_test_set.json (Action 3)
   - 50 requests across 7 intent classes
   - Run baseline accuracy against current keyword classifier

7. Run dual-mode smoke test
   - With broker: full enrichment path
   - Without broker: fallback path
   - Both must pass
```

---

## 8. Kill List Check — Patterns This Design Avoids

| Banned Pattern | Status |
|:---|:---|
| Monolithic agent with >10 skills | ✅ Broker has exactly 3: classify, score, route |
| NATS used for persistent state | ✅ Broker stores nothing. NATS is request-reply only |
| REST-only stateless inference | ✅ Not applicable — broker uses NATS |
| Cloud GPU rental | ✅ Not applicable — broker is CPU-first |
| >$40/month fixed cloud spend | ✅ Broker runs on Node A, zero cloud cost |
| 30B+ models on M4 Pro | ✅ Not applicable — broker uses rule-based scoring |

---

> [!NOTE]
> This document was reviewed and **approved by the operator on 2026-03-06T13:25 PST**. It is now the locked interface contract for Phase B broker extraction. Codex should implement against it without deviation. The canonical reference for any conflicts remains `config/swarm/BUILD_BLUEPRINT_2026-03-06.md`.
