# 🌐 Heiwa Universe

Canonical monorepo for the **Heiwa Swarm** — a sovereign, 24/7 autonomous AI orchestration mesh.

## Current Phase: B — Broker Extraction

| Phase | Name | Status |
|:------|:-----|:-------|
| **A** | Smoke & Barrier | ✅ Closed |
| **B** | Broker Extraction | 🔨 Active |
| C | Policy & Budget | Planned |
| D | SpacetimeDB Migration | Planned |
| E | Multi-Node Scaling | Planned |
| F | Surface Expansion | Planned |

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                        Ingress Surfaces                         │
│    Discord (Messenger)  ·  CLI (terminal_chat)  ·  API / Web    │
└──────────────────────────────┬──────────────────────────────────┘
                               │ NATS
                    ┌──────────▼──────────┐
                    │       SPINE         │
                    │  Auth Gate · State  │
                    │  Owner · Dispatch   │
                    └──────────┬──────────┘
                               │ NATS request-reply
                    ┌──────────▼──────────┐
                    │      BROKER         │  ← Phase B (building)
                    │  Classify · Score   │
                    │  Route · Enrich     │
                    └──────────┬──────────┘
                               │ NATS
              ┌────────────────┼────────────────┐
              ▼                ▼                ▼
     ┌────────────┐   ┌────────────┐   ┌────────────┐
     │  Executor  │   │   Codex    │   │  OpenClaw  │
     │  (Worker)  │   │ (Builder)  │   │(Strategist)│
     └────────────┘   └────────────┘   └────────────┘
```

**Key Principle:** Broker is an enricher, not a gate. If it's down, Spine falls back to inline processing. The swarm never halts.

## Repo Structure

```
heiwa/
├── apps/
│   ├── heiwa_hub/              # Core swarm — agents, cognition, boot
│   │   ├── agents/             # Spine, Executor, Messenger, Telemetry, Broker
│   │   ├── cognition/          # IntentNormalizer, RiskScorer, Planner, LLM
│   │   ├── actions/            # Smoke tests, operational scripts
│   │   └── tests/              # Classifier test set, scorer tests
│   ├── heiwa_cli/              # Interactive shell + telemetry footer
│   └── heiwa_web/              # Public visibility layer
├── packages/
│   ├── heiwa_sdk/              # Config, vault, DB, cognition engine
│   ├── heiwa_protocol/         # NATS subjects, payload keys (shared contract)
│   ├── heiwa_identity/         # Node UUID, identity management
│   └── heiwa_ui/               # Terminal UI components (Rich)
├── config/
│   └── swarm/                  # Build blueprint, broker design review, ai_router
├── AGENTS.md                   # Agent engineering reference (read this)
├── SOUL.md                     # System identity and behavior
├── CODEX_HANDOFF_PHASE_B.md    # Current builder handoff
└── requirements.txt
```

## Compute Classes

| Class | Name | Where | Models | Use Case |
|:------|:-----|:------|:-------|:---------|
| 1 | CPU-First | Local (Node A) | ≤7B quantized | Chat, status, simple tasks |
| 2 | GPU-Justified | Local (Node A) | ≤14B quantized | Code gen, analysis |
| 3 | Premium Remote | SiliconFlow/Cerebras | DeepSeek-R2, Gemini | Research, long-context |
| 4 | Cloud Persistence | Railway | Infra ops | Deploy, operate, monitor |

## Quick Start

```bash
# Setup
cd ~/heiwa
source .venv/bin/activate
export PYTHONPATH=$(pwd)/packages/heiwa_sdk:$(pwd)/packages/heiwa_protocol:$(pwd)/packages/heiwa_identity:$(pwd)/packages/heiwa_ui:$(pwd)/apps

# Start NATS
nats-server -js &

# Start the hub
python -m apps.heiwa_hub.main

# Run tests
python apps/heiwa_hub/tests/test_intent_classifier.py   # 50 cases, need ≥95%
python apps/heiwa_hub/tests/test_risk_scorer.py          # 21 cases
python apps/heiwa_hub/actions/smoke_test_discord.py      # Full bus + Discord
```

## Key Documents

| Document | Purpose |
|:---------|:--------|
| `AGENTS.md` | Agent engineering reference — architecture, topology, constraints |
| `SOUL.md` | System identity and behavioral guidelines |
| `CODEX_HANDOFF_PHASE_B.md` | Phase B implementation instructions for Codex |
| `config/swarm/BUILD_BLUEPRINT_2026-03-06.md` | Current build plan and phase targets |
| `config/swarm/BROKER_DESIGN_REVIEW.md` | Approved broker extraction interface contract |

---

*"Be genuinely helpful, not performatively helpful. Have opinions. Actions speak louder than filler words."* — **SOUL.md**
