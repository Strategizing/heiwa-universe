# Heiwa Build Blueprint Sync

Date: 2026-03-06
Source report: `/Users/dmcgregsauce/Downloads/heiwa_build_report.docx`

This file is the repo-local operating digest for the March 6, 2026 Heiwa build report. It exists so Codex, Gemini CLI, Antigravity, and the Heiwa repo all point at the same local manual.

## Non-Negotiables

- State layer: SpacetimeDB.
- Cloud host: Railway.
- Transport layer: HTTP/WebSocket ingress with in-process local bus for co-located agents and worker sessions.
- Canonical monorepo: `/Users/dmcgregsauce/heiwa`.
- Local-first execution for private or sovereign tasks.
- No dedicated cloud GPU rental.
- No stateless REST-only inference for multi-turn agentic sessions.
- No 30B+ models on the M4 Pro.
- No architecture that requires an external message broker for core ingress or durable state.
- Monthly fixed cloud floor target stays under `$40`.
- WebSocket-first inference for all cloud-based multi-turn agentic sessions.
- MCP Tool Shed: all agent-tool connectivity routes through MCP. No hard-coded API integrations in agent logic.
- Mandatory finalizers: every autonomous service execution must include resource cleanup. No leaked DB connections, sandboxes, or temp files.
- E2B sandboxes for untrusted code execution. LLM-generated code never runs on host infrastructure.
- Hardware constraints documented in `config/swarm/HARDWARE_CONSTRAINTS.md`.

## Hardware Topology

- Node A: MacBook M4 Pro, 24 GB unified memory.
  Role: orchestrator, CLI terminal, primary reasoning core, local model host.
  Preferred local models: `Llama 4 Scout` Q4_K_M GGUF, `GLM-4.7-Flash` Q4_K_M GGUF.
- Node B: Ryzen 7 7700X, RTX 3060 12 GB VRAM, 32 GB RAM.
  Role: headless GPU worker for embeddings, reranking, browser automation, media generation, local GPU inference.
  Preferred local models: `Qwen2.5-Coder-7B` Q4, `all-MiniLM-L6-v2`, `SDXL-Turbo` or `Flux-Schnell`.
- Cloud: Railway for public APIs, schedulers, status/docs surfaces, and SpacetimeDB hosting. Use E2B sandboxes for untrusted code execution.

## Four-Class Execution Model

- Class 1 CPU-first:
  Shell execution, file operations, Git operations, parsing, assembly, linting, audit checks.
- Class 2 GPU-justified:
  Local LLM inference, embeddings, vector reranking, image generation, audio processing.
- Class 3 Premium Remote:
  Complex reasoning, hard debugging, strategy planning, adversarial review, long-context work.
- Class 4 Cloud Persistence:
  Webhooks, schedulers, Railway deploys, Cloudflare Workers, status APIs, notifications.

## Agent Specialization

- Codex:
  Class 3 builder specialist for code generation, implementation, and PR preparation.
- Gemini CLI:
  Class 3 research and long-context specialist.
- Antigravity:
  Class 3 strategic proposal and adversarial review specialist.
- All agents stay narrow.
  Do not design monolithic agents with more than about 10 skills.

## Routing Rules

- Privacy-first gate:
  If `privacy_level = sovereign`, force Class 1 or 2 only.
- Cost gate:
  If the hourly or monthly budget is near exhaustion, downgrade or queue Class 3 work.
- Latency gate:
  If provider latency breaches SLA, force local execution for the next window.
- Provider order:
  Cheapest acceptable route first. Prefer local tooling, then local models, then wholesale API providers, then premium endpoints only when justified.
- Cloud inference overflow:
  Use API providers, not rented cloud GPUs.

## Boot Sequence

1. Read `/Users/dmcgregsauce/heiwa/SOUL.md`.
2. Read `/Users/dmcgregsauce/heiwa/AGENTS.md`.
3. Read `/Users/dmcgregsauce/heiwa/config/swarm/BUILD_BLUEPRINT_2026-03-06.md`.
4. Read `/Users/dmcgregsauce/heiwa/config/swarm/ai_router.json`.
5. Read `/Users/dmcgregsauce/heiwa/config/identities/profiles.json`.
6. Verify authenticated hub ingress and worker session transport before accepting work.
7. Verify SpacetimeDB connectivity before accepting work.
8. Broadcast initial node heartbeat with compute capabilities.

## Kill List

- Dedicated cloud GPU rental above commodity API pricing.
- Monolithic general-purpose agents.
- REST-only multi-turn agentic inference.
- 30B+ models on the M4 Pro.
- Kubernetes or multi-region cloud clusters for this operating scale.
- Any pattern that pushes fixed cloud spend above the report target.

## Transitional Note

The March 6 blueprint is the target operating model. Some runtime surfaces are still transitional today. When current implementation and blueprint differ, update configs and docs toward the blueprint without inventing fictional runtime state.
