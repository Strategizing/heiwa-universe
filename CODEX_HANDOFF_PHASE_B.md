# CODEX HANDOFF — Phase B Broker Extraction

> **From:** Antigravity (Class 3 — Strategic Review & Design)
> **To:** Codex (Builder)
> **Date:** 2026-03-06
> **Canonical Design:** `config/swarm/BUILD_BLUEPRINT_2026-03-06.md`

---

## What Just Happened (Read This First)

Antigravity completed four actions today. You're picking up from a clean state with all foundation work done:

| Action | What | Status | Commit |
|:---|:---|:---|:---|
| 1 | Discord smoke test (Phase A closure) | ✅ Passed | `f3fcf3c` |
| 2 | Broker extraction design review | ✅ Approved & locked | See below |
| 3 | Intent classifier test set (50 cases) | ✅ 100% baseline | `9d2844f` |
| 4 | Risk scorer v1 (rule-based) | ✅ 21/21 tests pass | `0ee873f` |

**Phase A is fully closed.** The bus-level and Discord smoke tests both pass. You are now building Phase B.

---

## Your Mission: Build the Broker Agent

Extract the `heiwa-broker` service from the monolithic Spine. The broker is a pure NATS compute agent that enriches task envelopes with intent classification, risk scoring, and compute routing — then hands them back to Spine for dispatch.

### Design Principle (Memorize This)

> **The broker is an enricher, not a gate.** If it's down, the swarm falls back to inline processing. The broker never halts the system.

---

## Environment Setup

```bash
cd /Users/dmcgregsauce/heiwa

# Python venv (already created)
source .venv/bin/activate

# PYTHONPATH for monorepo packages
export PYTHONPATH=/Users/dmcgregsauce/heiwa/packages/heiwa_sdk:/Users/dmcgregsauce/heiwa/packages/heiwa_protocol:/Users/dmcgregsauce/heiwa/packages/heiwa_identity:/Users/dmcgregsauce/heiwa/packages/heiwa_ui:/Users/dmcgregsauce/heiwa/apps

# Start NATS (required for any agent work)
nats-server -js &

# Run the hub (boots Spine + Executor + Telemetry + Messenger)
python -m apps.heiwa_hub.main

# Run smoke test (in separate terminal)
python apps/heiwa_hub/actions/smoke_test_discord.py
```

**Important:** The `.env` and `.env.worker.local` files use `load_dotenv(override=True)` in priority order via `heiwa_sdk.config.load_swarm_env()`. Worker local overrides base `.env`. Don't fight the config system — modify `.env` or `.env.worker.local` directly when you need to change env vars.

---

## Locked Interface Contract

The full design review with all interface details is at:
**`config/swarm/BROKER_DESIGN_REVIEW.md`**

### The Short Version

```
Spine authenticates → publishes to heiwa.broker.route (NATS request-reply)
Broker enriches    → replies with intent_class, risk_level, compute_class, steps
Spine dispatches   → to heiwa.tasks.exec (same as today)
```

If broker doesn't reply within 5 seconds, Spine falls back to inline processing (current behavior). The system never stops.

### Envelope Contract

```python
# Spine sends:
class BrokerRouteRequest:
    request_id: str       # MUST be echoed back unchanged
    task_id: str
    raw_text: str
    sender_id: str
    source_surface: str   # "discord" | "cli" | "api" | "web"
    response_channel_id: int | str
    response_thread_id: int | str | None
    auth_validated: bool  # always True
    timestamp: float
    envelope_version: str # "2026-03-06"

# Broker replies:
class BrokerRouteResult:
    request_id: str       # MUST match the request
    task_id: str
    intent_class: str
    risk_level: str       # low | medium | high | critical
    compute_class: int    # 1-4
    assigned_worker: str
    requires_approval: bool
    steps: list[dict]
    normalization: dict
    raw_text: str
    response_channel_id: int | str
    response_thread_id: int | str | None
    envelope_version: str # echo back
```

**Critical invariant:** `BrokerRouteResult.request_id` MUST equal `BrokerRouteRequest.request_id`. Spine rejects mismatches.

### State Permissions

- **Broker has ZERO database access.** It's pure NATS in/out.
- **Spine retains all DB writes.** This is non-negotiable for Phase B.
- Phase D promotion path: flip `HEIWA_BROKER_STATE_MODE` env var from `nats_only` to `stdb_direct`. No code changes needed.

---

## Implementation Steps (Do These In Order)

### Step 1: Create `apps/heiwa_hub/agents/broker.py`

- Extends `BaseAgent` (from `apps/heiwa_hub/agents/base.py`)
- Subscribes to `heiwa.broker.route` via NATS
- Imports and uses:
  - `IntentNormalizer` from `apps/heiwa_hub/cognition/intent_normalizer.py`
  - `RiskScorer` from `apps/heiwa_hub/cognition/risk_scorer.py` (already built)
  - `ComputeRouter` from step 2 below
- Receives `BrokerRouteRequest`, runs enrichment pipeline, replies with `BrokerRouteResult`
- Must validate `envelope_version` and echo `request_id` unchanged

**Pattern reference:** Look at how `ExecutorAgent` handles NATS messages in `apps/heiwa_hub/agents/executor.py`. Same pattern — subscribe, process, reply.

### Step 2: Create `apps/heiwa_hub/cognition/compute_router.py`

- Maps `(intent_class, risk_level)` → `compute_class` (1-4) + `assigned_worker` hint
- Reads `config/swarm/ai_router.json` for model registry and compute class definitions
- Privacy constraint: sovereign data MUST route to class 1 or 2 (local compute only)
- Reference the 4 execution tiers from `BUILD_BLUEPRINT_2026-03-06.md`:
  - Class 1: CPU-first (local, ≤7B models)
  - Class 2: GPU-justified (local, ≤32B models)
  - Class 3: Premium remote (Gemini, Claude — context-heavy research)
  - Class 4: Cloud persistence (Railway, infra ops)

### Step 3: Modify `apps/heiwa_hub/agents/spine.py`

- After auth validation in `handle_request()`, attempt NATS request to `heiwa.broker.route`
- Use `self.nc.request("heiwa.broker.route", payload, timeout=5.0)`
- On success: use broker's enriched envelope, dispatch with `DISPATCHED_PLAN`
- On timeout: fall back to existing inline processing, dispatch with `DISPATCHED_FALLBACK`
- Validate `request_id` on the response matches what was sent
- Preserve ALL existing behavior as the fallback path — do not remove any current code

### Step 4: Modify `apps/heiwa_hub/main.py`

- Add `BrokerAgent` to the boot sequence
- Make it optional: controlled by `HEIWA_ENABLE_BROKER` env var (default: `true`)
- Same pattern as how `MessengerAgent` is optionally booted (lines 92-101 of current main.py)

### Step 5: Run Dual-Mode Smoke Test

- Run `smoke_test.py` with `HEIWA_ENABLE_BROKER=true` → must pass
- Run `smoke_test.py` with `HEIWA_ENABLE_BROKER=false` → must also pass
- Run `smoke_test_discord.py` with broker enabled → must pass

---

## Verification Gates (No Step Advances Without Its Gate Passing)

| Step | Gate Test | Test Artifact | Pass Criteria |
|:---|:---|:---|:---|
| 1 | Bus smoke test with risk scoring in broker | `apps/heiwa_hub/actions/smoke_test.py` | Exit 0. `DISPATCHED_PLAN` payload has `risk_level`. |
| 2 | 50-request classifier through broker | `apps/heiwa_hub/tests/test_intent_classifier.py` | ≥95% accuracy (≥48/50). |
| 3 | Compute class assignment | `apps/heiwa_hub/tests/test_compute_router.py` (you create this) | All 7 intent classes → correct compute_class. Privacy=sovereign → class 1/2. |
| 4 | Full Discord smoke with broker | `apps/heiwa_hub/actions/smoke_test_discord.py` | All 4 bus transitions. Bot posts. <30s. |
| **Final** | **Dual-mode** | Both smoke tests | Pass with `HEIWA_ENABLE_BROKER=true` AND `false`. |

---

## Existing Code You Need to Know

### Files You'll Read

| File | What It Does | Why You Care |
|:---|:---|:---|
| `apps/heiwa_hub/agents/base.py` | `BaseAgent` ABC with NATS connect/listen/speak | Your broker extends this |
| `apps/heiwa_hub/agents/spine.py` | Monolithic orchestrator — auth, planning, dispatch | You're modifying this |
| `apps/heiwa_hub/agents/executor.py` | Processes dispatched tasks | Pattern reference for NATS handling |
| `apps/heiwa_hub/cognition/intent_normalizer.py` | Keyword-based intent classifier | Broker imports this |
| `apps/heiwa_hub/cognition/risk_scorer.py` | Rule-based risk scoring | **Already built.** Broker imports this |
| `apps/heiwa_hub/cognition/planner.py` | Step decomposition from raw text | Moves to broker in Step 4 of extraction |
| `apps/heiwa_hub/main.py` | Hub boot sequence | You add BrokerAgent here |
| `apps/heiwa_hub/envelope.py` | Token/payload extraction | Spine auth uses this |
| `packages/heiwa_protocol/heiwa_protocol/protocol.py` | NATS subjects, payload keys | Add new broker subjects here |

### Files You'll Create

| File | Purpose |
|:---|:---|
| `apps/heiwa_hub/agents/broker.py` | BrokerAgent — NATS request-reply enrichment |
| `apps/heiwa_hub/cognition/compute_router.py` | (intent, risk) → compute_class mapping |
| `apps/heiwa_hub/tests/test_compute_router.py` | Gate 3 verification test |

### Files You'll Modify

| File | Change |
|:---|:---|
| `apps/heiwa_hub/agents/spine.py` | Add broker request-reply with 5s timeout fallback |
| `apps/heiwa_hub/main.py` | Add optional BrokerAgent to boot |
| `packages/heiwa_protocol/heiwa_protocol/protocol.py` | Add `BROKER_ROUTE` subject to the Subject enum |

---

## Hard Constraints (From BUILD_BLUEPRINT)

1. **No 30B+ models on M4 Pro.** Broker uses rule-based scoring, not LLM inference.
2. **NATS for ephemeral events only.** Don't use NATS as persistent state.
3. **Cloud spend < $40/month.** Broker runs on Node A (local), zero cloud cost.
4. **Broker has no database access.** Pure NATS compute.
5. **Broker must be stateless.** No caching, no session memory, no local state files.
6. **Preserve fallback.** Every change to Spine must keep inline processing as the fallback path.
7. **`request_id` correlation.** Every broker response must echo the request's `request_id`.

---

## What NOT To Do

- ❌ Don't give broker database credentials
- ❌ Don't make broker a hard dependency (timeout fallback is mandatory)
- ❌ Don't remove any existing Spine behavior (it's the fallback path)
- ❌ Don't use gRPC or REST between Spine and broker (NATS only)
- ❌ Don't build the Task Planner extraction yet (that's Step 4 of extraction, after classifier and router are stable)
- ❌ Don't touch SpacetimeDB (that's Phase D)

---

## Commit Convention

Use descriptive commit messages referencing the Phase B step:

```
Phase B Step 1: BrokerAgent with NATS request-reply enrichment
Phase B Step 2: ComputeRouter maps (intent, risk) → compute_class  
Phase B Step 3: Spine broker integration with 5s timeout fallback
Phase B Step 4: BrokerAgent added to hub boot sequence
Phase B Step 5: Dual-mode smoke test — pass with and without broker
```

---

## Questions? Read These Files

1. Full design review: `config/swarm/BROKER_DESIGN_REVIEW.md`
2. Build blueprint: `config/swarm/BUILD_BLUEPRINT_2026-03-06.md`
3. System identity: `SOUL.md`
4. Agent conventions: `AGENTS.md`
