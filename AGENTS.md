# AGENTS.md — Heiwa Engineering Agent Reference

> **Last Updated:** 2026-03-06 by Antigravity
> **Canonical Source:** This file is the single reference for any AI agent working on Heiwa.
> **Rule:** Read `SOUL.md` first (identity), then this file (how to work), then your handoff document (what to build).

---

## 1. Boot Sequence

Before doing any work on Heiwa:

1. **Read `SOUL.md`** — who you are, how you behave
2. **Read this file** — architecture, conventions, constraints
3. **Read your handoff document** (e.g., `CODEX_HANDOFF_PHASE_B.md`) — your specific task
4. **Read `config/swarm/BUILD_BLUEPRINT_2026-03-06.md`** — the current build plan and phase targets
5. **Check `.env`** — verify environment variables are correct for your task

Do not ask permission for these reads. Just do them.

---

## 2. Heiwa Architecture (Current State)

### System Overview

Heiwa is a sovereign AI orchestration swarm. It receives tasks from multiple surfaces (Discord, CLI, API), classifies and scores them, plans execution steps, dispatches work to agent executors, and delivers results back to the requesting surface.

### Agent Registry

| Agent | File | Role | State Access |
|:---|:---|:---|:---|
| **Spine** | `apps/heiwa_hub/agents/spine.py` | Orchestrator. Auth gate, planning, dispatch. Single state owner. | DB read/write |
| **Executor** | `apps/heiwa_hub/agents/executor.py` | Worker. Receives planned steps, runs LLM inference or tools, returns results. | NATS only |
| **Messenger** | `apps/heiwa_hub/agents/messenger.py` | Discord gateway. Ingests commands, delivers results to channels/threads. | Discord API |
| **Telemetry** | `apps/heiwa_hub/agents/telemetry.py` | Monitoring. CPU/RAM stats, node heartbeats, fleet health. | NATS only |
| **Broker** *(Phase B)* | `apps/heiwa_hub/agents/broker.py` | Enricher. Intent → risk → compute class. Pure NATS, zero state. | NATS only |
| **Codex** | `apps/heiwa_hub/agents/codex.py` | Builder proposal agent. Listens for code tasks, produces implementations. | NATS only |
| **OpenClaw** | `apps/heiwa_hub/agents/openclaw.py` | Strategist proposal agent. Listens for strategy tasks, produces proposals. | NATS only |

All agents extend `BaseAgent` from `apps/heiwa_hub/agents/base.py`.

### Cognition Pipeline

| Module | File | Purpose |
|:---|:---|:---|
| **IntentNormalizer** | `apps/heiwa_hub/cognition/intent_normalizer.py` | Keyword-based intent classification (13 intent classes) |
| **RiskScorer** | `apps/heiwa_hub/cognition/risk_scorer.py` | 3-layer risk scoring (intent defaults → keyword escalators → surface trust) |
| **LocalTaskPlanner** | `apps/heiwa_hub/cognition/planner.py` | Step decomposition from raw text into executable plan |
| **ApprovalGate** | `apps/heiwa_hub/cognition/approval.py` | Human-in-the-loop for high-risk operations |
| **CognitionEngine** | `apps/heiwa_hub/cognition/llm_local.py` | Tiered LLM inference (local Ollama → remote models) |

### NATS Subject Topology

```
# Task Lifecycle
heiwa.core.request          # Legacy ingress (User/API → Spine)
heiwa.tasks.new             # V2 ingress (Discord/Gateway → Spine)
heiwa.tasks.exec            # Dispatch (Spine → Executor)
heiwa.tasks.exec.result     # Results (Executor → Messenger)
heiwa.tasks.status          # Lifecycle events (all agents emit, Messenger consumes)
heiwa.tasks.progress        # Real-time progress during long tasks

# Broker (Phase B — being built)
heiwa.broker.route          # Spine → Broker (send envelope for enrichment)
heiwa.broker.route.result.* # Broker → Spine (reply with enriched envelope)

# Node Management
heiwa.node.heartbeat        # Liveness pulses
heiwa.node.telemetry        # Resource usage stats
heiwa.node.register         # New node announcement

# Mesh (V2 Decentralized)
heiwa.mesh.capability.broadcast
heiwa.mesh.task.bid
heiwa.mesh.task.claim

# Logging
heiwa.log.info
heiwa.log.error
heiwa.log.thought
```

### Package Structure

```
heiwa/
├── apps/heiwa_hub/           # Main application
│   ├── main.py               # Hub boot sequence
│   ├── agents/               # All agent implementations
│   ├── cognition/            # Intent, risk, planning, LLM
│   ├── actions/              # Smoke tests, scripts
│   └── tests/                # Test sets and runners
├── packages/
│   ├── heiwa_sdk/            # Config, vault, DB, cognition engine
│   ├── heiwa_protocol/       # NATS subjects, payload keys (shared contract)
│   ├── heiwa_identity/       # Node UUID, identity management
│   └── heiwa_ui/             # Terminal UI components
├── config/
│   ├── swarm/                # Build blueprint, topology docs
│   └── identities/           # Agent identity profiles
├── .env                      # Base environment variables
├── .env.worker.local         # Worker-specific overrides (higher priority than .env)
└── requirements.txt          # Python dependencies
```

### Config Loading Priority

`heiwa_sdk.config.load_swarm_env()` loads in this order (each overrides the previous):

1. `.env` (base)
2. `.env.worker.local` (worker overrides)
3. `vault.env` (secrets — highest priority)

**If a variable is set in both `.env` and `.env.worker.local`, the worker value wins.** This has bitten agents before — always check both files when debugging config issues.

---

## 3. Intent Taxonomy (13 Classes)

| Intent | Risk Default | Approval | Target Runtime | Use Case |
|:---|:---|:---|:---|:---|
| `build` | medium | No | macbook/codex | Create, implement, code, script |
| `deploy` | high | Yes | railway/heiwa_ops | Ship to staging/production |
| `operate` | high | Yes | railway/heiwa_ops | Fix, debug, incident response |
| `files` | high | Yes | macbook/codex | File system mutations |
| `mesh_ops` | medium | Yes | macbook/codex | Node sync, mesh topology |
| `self_buff` | high | Yes | macbook/codex | Self-improvement, optimization |
| `chat` | low | No | railway/ollama | Casual conversation |
| `automate` | medium | Yes | railway/n8n | Workflows, cron, triggers |
| `strategy` | medium | No | both/openclaw | Design, architecture, roadmap |
| `research` | low | No | both/openclaw | Analyze, compare, investigate |
| `audit` | low | No | both/heiwa_ops | Verify, scan, validate, test |
| `media` | low | No | both/ollama | Image, video, audio, visual |
| `status_check` | low | No | railway/ollama | System health, uptime |

Fallback intent: `general` (low risk, no approval, ollama).

---

## 4. Hard Constraints (Violating These Is a Bug)

1. **No 30B+ models on M4 Pro.** Local inference uses ≤14B quantized models only.
2. **No cloud GPU rental inside Heiwa.** Use SiliconFlow, Cerebras, or other inference APIs.
3. **Cloud spend < $40/month total.** Budget is sovereign — no runaway costs.
4. **NATS is ephemeral.** Do not use NATS as persistent state or durable storage.
5. **Spine is the single state owner.** No other agent writes to the database in Phase B.
6. **Broker is stateless.** No caching, no session memory, no local files.
7. **Every Spine change must preserve fallback.** Inline processing is the safety net.
8. **Secrets stay in vault.** Never hardcode tokens. Never log tokens. Use `vault.env` or env vars.
9. **Privacy-sovereign data stays local.** Route to compute class 1 or 2, never to cloud.

---

## 5. Development Workflow

### Running the Hub

```bash
cd /Users/dmcgregsauce/heiwa
source .venv/bin/activate
export PYTHONPATH=$(pwd)/packages/heiwa_sdk:$(pwd)/packages/heiwa_protocol:$(pwd)/packages/heiwa_identity:$(pwd)/packages/heiwa_ui:$(pwd)/apps

# Start NATS
nats-server -js &

# Start the hub
python -m apps.heiwa_hub.main
```

### Running Tests

```bash
# Intent classifier (50 cases, ≥95% threshold)
python apps/heiwa_hub/tests/test_intent_classifier.py

# Risk scorer (21 cases)
python apps/heiwa_hub/tests/test_risk_scorer.py

# Discord smoke test (requires hub + NATS running)
python apps/heiwa_hub/actions/smoke_test_discord.py
```

### Commit Convention

```
Phase [X] Step [N]: [Short description]

[Longer description if needed]
```

Example: `Phase B Step 1: BrokerAgent with NATS request-reply enrichment`

### Environment Variables That Matter

| Variable | What It Does | Where Set |
|:---|:---|:---|
| `HEIWA_AUTH_TOKEN` | Digital Barrier — authenticates all inbound tasks | `.env` + `.env.worker.local` |
| `HEIWA_ENABLE_MESSENGER` | Enables Discord gateway agent | `.env` |
| `HEIWA_ENABLE_BROKER` | Enables broker enrichment agent (Phase B) | `.env` |
| `NATS_URL` | NATS server connection string | `.env` |
| `DISCORD_BOT_TOKEN` | Discord bot credentials | `.env` |
| `HEIWA_DISCORD_SMOKE_CHANNEL_ID` | Channel for smoke test verification | `.env` |
| `HEIWA_BROKER_STATE_MODE` | `nats_only` (Phase B) or `stdb_direct` (Phase D) | `.env` |

---

## 6. Phase Map

| Phase | Name | Status | Scope |
|:---|:---|:---|:---|
| **A** | Smoke & Barrier | ✅ CLOSED | Bus test, Discord test, auth validation |
| **B** | Broker Extraction | 🔨 ACTIVE | Extract intent/risk/routing from Spine into broker |
| **C** | Policy & Budget | Planned | Objective function, cost guards, provider rotation |
| **D** | SpacetimeDB | Planned | Replace SQLite/Postgres with SpacetimeDB |
| **E** | Multi-Node | Planned | Scale across Node A + Node B |
| **F** | Surface Expansion | Planned | Voice, Slack, web dashboard ingress |

---

## 7. Safety & Trust

- **Read-only first.** Inspect before you mutate. `git status` before `git commit`.
- **Secrets are redacted.** Never log, echo, or commit tokens, passwords, or API keys.
- **Write-gated autonomy.** Propose evidence before high-impact mutations. Commit messages explain what and why.
- **`trash` > `rm`.** Recoverable beats gone forever.
- **Ask when uncertain.** If you're unsure about a design decision, stop and ask.

---

## 8. Current Handoff Documents

| Document | Agent | Scope |
|:---|:---|:---|
| `CODEX_HANDOFF_PHASE_B.md` | Codex | Broker extraction implementation |

When you receive a new handoff, it will appear in the repo root. Read it before starting work.

---

_This file is maintained by the operator and build agents. If you update it, commit the change with a clear message explaining what was modified._
