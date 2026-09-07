# Product Contract

**Status:** Personal-first product contract for the enterprise-grade Heiwa stack.

This document defines what Heiwa is allowed to present as product. `HEIWA.md`
remains the architecture authority. `PRODUCT_SURFACE.md` remains the tracked-path
classification authority. This file names the real products, services, and feature
families.

The modular resource model is defined in [`capability-fabric.md`](capability-fabric.md).

## First Customer

The first customer is Devon.

The first winning product is not a generic public AI dashboard. It is the operator
system that lets one person safely use local models, provider CLIs, hosted models,
computer use, messaging surfaces, files, and personal workflows through one
governed runtime.

Personal-first does not mean hobby-grade. It means the system must work for one
high-trust operator before it claims team or enterprise maturity.

## Product Set

### 1. Heiwa Runtime

The installed `heiwa` runtime is the primary execution product.

It owns:

- local install, doctor, auth, provider, and session UX
- provider and local-model discovery
- routing policy and budget-aware execution
- session history, receipts, artifacts, and evidence
- local process supervision and side-effect execution
- approval classes for shell, browser, computer-use, messaging, and file actions
- offline-local and online-local modes

This is the product that turns Codex, Claude Code, Gemini CLI, Antigravity, Ollama,
MCP tools, and future agents into one operator surface. Those providers remain
provider-owned; Heiwa owns routing, evidence, policy, and local UX.

### 2. Heiwa.app

`Heiwa.app` is the primary user input/display application for the same runtime.
It is the place where the user sees Heiwa thinking, asking, doing, staging, and
returning evidence.

Normal installs must place it under the user's HOME-local Heiwa root:
`~/.heiwa/app/Heiwa.app` on macOS, or the platform-equivalent app path on other
machines. Today that path is a launcher bridge over the local app runtime. The
target is a native wrapper over the same client/runtime contract, not a
privileged second control plane or a disconnected admin site.

The primary user experience is a single input/output conversation with Heiwa.
The primary visual experience is Home as a pinned ops board: live terminal
instances, worker panes, sub-app servers, approvals, receipts, and local run
state are visible without hover. Feature icons behave like a dock for quick
preview/focus, but Calendar, Mail, Finance, Social, AI, Files, Browser, and
terminal work are in-app panes connected to the same runtime brain and evidence
policy. Pages such as Inbox, Providers, History, Traces, Memory, and Status are
inspectors for the same runtime state; they are not separate places the user
must mentally route work. The user asks or responds in one thread. Heiwa uses
background context from connected surfaces, then reports decisions, staged
actions, receipts, and blockers back through that thread.

It should expose:

- the main Heiwa/user conversation stream
- Home pinned ops state for what Heiwa is doing now
- multiplexer panes/windows for conversation, workers, terminals, approvals,
  receipts, sub-app servers, and connected-surface context
- per-sub-app agent profiles: relevant skills, allowed tools, personalization,
  risk class, and evidence behavior
- packaged app bridges for local-only pane/herd state, Deno sub-app sidecars,
  and terminal daemon state; browser preview is development support, not the
  product runtime
- account and provider connection state
- personalization for skills, rules, preferences, connectors, and notification
  behavior
- auto-managed projects that Heiwa infers from durable work, not manual ticketing
- devices and runtime health
- task, run, and receipt history
- approvals and risk classes
- routing policy visibility without forcing normal users to pick models manually
- public-safe status and diagnostics
- connected-surface context from browser, mail, calendar, messages, machine
  resources, computer use, and third-party integrations
- unified Calendar and Communications surfaces that stage external writes/replies through approvals and receipts

The browser console is secondary and user-scoped. It is a pseudo-backend/admin
surface for the specific user/machine: advanced settings, personalization,
projects, telemetry overview, connector setup, and links into dashboard/app
settings. It should not become the everyday operator path.

It must not hold raw provider secrets, bypass runtime policy, or become the place
where privileged automation logic lives.

### 3. Heiwa Distribution Backbone

The current backbone keeps runtime authority on the operator machine:

| Service             | Product role                                                     |
| ------------------- | ---------------------------------------------------------------- |
| GitHub              | source, CI, release artifacts, installer, public repo front page |
| Cloudflare          | DNS and static public shell/installer delivery                    |
| Local JSONL + Lance | canonical evidence plus derived local recall                     |

This is not a hosted control plane. Local runtimes execute side effects, hold
trust, and own durable state. GitHub evidence sync is planned and must remain
redaction-gated.

Current boundary: there is no hosted Rust service tier in the v0.1 topology.
The local runtime owns provider streams, shell work, local models, approvals,
and side effects. A hosted control plane can be reconsidered only after a later
stage proves the need and does not add a hidden inference middleman.

### 4. Heiwa Integration Registry

Integrations are capability lanes, not separate products by default.

pi-mono, Hermes-style agents, provider coding agents, and computer-use agents are
reference classes for what Heiwa coordinates. They are not brands Heiwa copies.

Initial lanes:

- major account providers: Apple, Google, Microsoft, GitHub
- provider CLIs: Claude Code, Codex, Gemini CLI, Antigravity
- local model runtimes: Ollama and later local/sovereign runners
- direct APIs and routers: OpenAI, Anthropic, Google, OpenRouter, LiteLLM-style adapters
- messaging ingress: Discord first where already present, iMessage only through a safe local bridge
- computer use: browser, desktop, files, shell, and app control through explicit approval classes
- MCP and local tools: registered tools with evidence and lease boundaries

Every integration must map into a trust class: provider adapter, tool, hook, or
reducer/policy. "BYOX" is only registration vocabulary.

Each account/tool/model integration must declare scopes, auth mode, resource map,
action schemas, lease rules, evidence hooks, and revocation behavior before it is
treated as product-grade.

## Service Boundaries

| Service boundary | Runs where        | Owns                                                                       | Must not own                              |
| ---------------- | ----------------- | -------------------------------------------------------------------------- | ----------------------------------------- |
| Local runtime    | Devon/user device | side effects, provider subprocesses, secrets, JSONL evidence, Lance recall | hosted authority or unredacted sync       |
| Cloudflare edge  | Cloudflare        | DNS records, static public shell, installer scripts                         | runtime state, binary authority, or privileged automation |
| GitHub           | GitHub            | source, CI, releases, install distribution                                 | live user state or private runtime memory |
| Public website   | Cloudflare Pages (shell), GitHub Pages (docs) | marketing, docs, install, public repo trust              | privileged control-plane mutations        |

## Feature Families

### P0: Devon-Useful Core

- install and doctor that reflect real local machine state
- provider connect/status for local, OAuth CLI, and API-key modes
- connector manifests for Apple, Google, Microsoft, GitHub, messaging, and computer-use lanes
- local-first routing that avoids provider-token tax when deterministic handling is enough
- invisible model/provider selection through an evolving capability/eval matrix rather than a normal-user model picker
- durable sessions, receipts, and evidence
- bounded loops with clear keep/discard behavior
- approval-gated shell, browser, file, and computer-use actions
- Heiwa.app dashboard over the same state, not a separate brain
- public install/docs/GitHub surface that does not overstate maturity

### P1: Personal Life Integration

- Discord and iMessage ingress as optional clients
- account-aware workflows across mail, calendar, files, repos, messages, browser, and desktop apps
- local reminders, recurring tasks, and monitors through explicit automations
- computer-use actions with previews, approvals, and receipts
- file, browser, app, and communication workflows routed through the same runtime
- per-surface trust policy: local CLI is higher trust than public chat ingress

### P2: Team/Enterprise Expansion

- org identity and policy
- device and fleet governance
- shared evidence and approvals
- tenant-scoped provider inventory and budgets
- compliance-ready audit exports
- hosted assist flows that do not bypass local/runtime trust rules

## Public Surface Contract

Safe public surfaces:

- `heiwa.ltd`: marketing, install, docs pointers
- GitHub repo front page: source, release, trust, issue/PR intake
- `docs.heiwa.ltd`: public docs
- `status.heiwa.ltd`: read-only health/status
- `app.heiwa.ltd`: safe client shell when authenticated and policy-backed

Unsafe as public product claims until proven:

- arbitrary remote computer-use execution
- raw Discord/iMessage command execution without trust policy
- provider-secret handling in browser or edge code
- equal execution parity across every provider
- team/enterprise governance before single-operator evidence works

## Non-Products

These are not standalone products right now:

- DREX as a public brand
- Lance or GitHub as an operator surface
- any hosted Rust service tier as the v0.1 product center
- Discord as the only interface
- a generic web IDE
- a giant research archive
- ungoverned "agent marketplace" surfaces

## Success Gate

Heiwa becomes real when Devon can ask for work through `heiwa`, Heiwa.app, or a
safe messaging ingress, and the system can move that work cleanly across all
three planes (see [Three Planes in `HEIWA.md`](../HEIWA.md#the-three-planes)):

**Intake**

1. classify the intent and risk
2. resolve the required account, data, tool, model, device, and agent material

**Execution**

3. pick the cheapest acceptable route
4. execute locally or through connected providers/subagents
5. ask for approval before risky side effects

**Evidence**

6. record evidence and receipts
7. expose the result through the same runtime/app state

Anything that does not move this gate forward is support infrastructure,
reference material, or slop until proven otherwise.
