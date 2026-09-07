# Local Self-Operation

This is the runtime contract for Heiwa on Devon's MacBook first. The same
contract must scale to each enrolled user machine without changing the product
model.

The goal is simple: the installed `heiwa` runtime should authenticate provider
CLIs through their owner-managed configs, read/write local state under
`~/.heiwa`, expose the cockpit on localhost, append durable JSONL evidence, and
derive local recall through Lance. Evidence sync to GitHub is planned but
disabled until a redaction and privacy boundary exists.

## Required Local Inputs

| Input                                   | Purpose                                           |
| --------------------------------------- | ------------------------------------------------- |
| `~/.heiwa/config.toml`                  | Runtime configuration                             |
| `~/.heiwa/accounts.json`                | Provider/account registry                         |
| `~/.heiwa/machine.json`                 | Local machine identity and capability manifest    |
| `~/.heiwa/state/`                       | Local runtime state, approvals, worker heartbeats |
| `~/.heiwa/evidence/`                    | Canonical local JSONL evidence journal            |
| `~/.claude/`, `~/.codex/`, `~/.gemini/` | Provider-owned auth and hook posture              |

## Boot Contract

`heiwa app start --port 7474` must:

1. Serve the cockpit and local API on `127.0.0.1`.
2. Report health at `/status/health`.
3. Write local app worker heartbeats under `~/.heiwa/state`.
4. Report provider, route, approval, worker, and hook posture without mutating provider-owned configs.
5. Keep running without public DNS, GitHub connectivity, or evidence sync.
6. Refresh `~/.heiwa/machine.json` with current host, OS, arch, install path, runtime version, and capability probes.
7. Adapt worker concurrency, polling cadence, and local-model use to machine load, battery, thermal state, and available runtimes.
8. Surface pending update or restart requirements without interrupting active work.
9. Require local runtime authentication for operator HTTP, turn submission,
   cancellation, and `/ws/v1/operator`; a localhost listener alone is not a
   trusted operator session.

## Install and Update Authority

GitHub and Cloudflare form the public install source, but they do not have the
same authority.

| Surface           | Authority                                                                                         |
| ----------------- | ------------------------------------------------------------------------------------------------- |
| GitHub repository | Canonical source code, tags, CI evidence, release artifacts, checksums, and install scripts       |
| GitHub Releases   | Canonical binary/archive distribution and version provenance                                      |
| Cloudflare        | Public edge, docs, install landing pages, update manifest cache, status, and future remote attach |
| Local machine     | Installed binary, local config, provider auth, local state, and user-approved side effects        |

Cloudflare may front or cache install/update material, but it must point back to
GitHub release identity and checksums. Cloudflare must not become a second
source of binary truth.

GitHub Releases are the authoritative public install and update path, including
on the operator MacBook. Local checkout promotion (`heiwa app update --source
checkout`) is reserved for development or recovery and must identify the exact
checkout commit in its receipt; it is not evidence of a public release.

`heiwa app update --dry-run` is the safe probe for the installed runtime and
defaults to GitHub Releases. It should report:

- installed version and path
- target version, channel, and release URL
- release commit or tag
- checksum/signature status when available
- whether restart is needed
- whether active tasks block restart

The runtime should prompt for update/restart when a newer compatible release is
detected, when cockpit assets are newer than the running server, or when a
schema/runtime boundary requires restart.

## Restart and Update Contract

Restart is an operator-visible state transition, not a silent side effect.

Default behavior:

1. Detect update or restart requirement.
2. Classify active work as `none`, `pausable`, or `blocking`.
3. Prompt the operator with target version, source, expected downtime, active tasks, and rollback path.
4. Apply update/restart within the user's authorization. A request that already
   includes the installed update/restart is sufficient; do not ask again for
   the same action. Verify active-work safety immediately before applying it.
5. Emit an evidence receipt with before/after versions and task handling.

Optional auto-restart is allowed only when explicitly enabled and one of these
conditions holds:

- no active tasks, no pending approvals, no external side effects in flight
- all active tasks are paused, leased work is checkpointed, and traces/events are flushed

Auto-restart must not run while a provider subprocess, file mutation, network
mutation, payment, booking, message send, or credential operation is in flight.
Those cases require an approval prompt.

If existing authorization explicitly covers interrupting that active work,
follow it and record the disposition. Otherwise stage the restart until the
work completes or the operator decides how it should be handled.

Pause-before-restart must:

1. Stop accepting new work.
2. Mark active tasks as paused with restart reason.
3. Close or renew leases deterministically.
4. Flush `~/.heiwa/state`, traces, logs, and evidence receipts.
5. Restart the runtime.
6. Rehydrate machine state and resume only tasks whose leases and approval policy still allow continuation.

## Machine Initialization and Adaptation

Each machine initializes as a local Heiwa node with its own capabilities. Heiwa
must assume N user machines over time, not one hardcoded owner path.

On first boot or install, the runtime should:

1. Create or refresh `~/.heiwa/machine.json`.
2. Record stable machine id, hostname, OS, arch, CPU/GPU class, memory, battery/thermal availability, install path, and runtime channel.
3. Discover local providers and CLIs without mutating provider-owned configs.
4. Discover local model runtimes such as Ollama.
5. Record machine identity locally; future cross-machine sync must be explicitly redaction-gated.
6. Write a boot receipt under local evidence state.

Adaptation rules:

- Battery or thermal pressure reduces background polling and pauses non-urgent work.
- Low memory or CPU load pressure reduces concurrency before degrading UX.
- Machines with strong local models should take cheap sovereign work first.
- Machines without local models should route through approved provider lanes.
- Machine-specific provider auth stays local and provider-owned.
- Cross-machine sync is planned through redacted evidence and machine identity; raw secrets never participate.

## Agentic Runtime Workflow

Use this workflow when an AI agent is developing, testing, or operating Heiwa.
The goal is to prove the current runtime, avoid stale localhost processes, and
leave no temporary process or file behind.

### 1. Understand before acting

Read in this order before architecture or runtime changes:

1. [`HEIWA.md`](../HEIWA.md) for canonical product truth.
2. [`AGENTS.md`](../AGENTS.md) for repo-specific agent rules.
3. This file for local boot, stop, and verification rules.

Classify the task as **Intake**, **Execution**, **Evidence**, or
out-of-scope before editing. If the work does not advance one of those planes,
defer it.

### 2. Probe without mutating

Start every runtime task with no-side-effect probes:

```bash
heiwa app update --dry-run
heiwa app runtime status --json
heiwa providers
```

When working from the checkout instead of the installed binary, prefer:

```bash
cargo run -q -p heiwa-shell --bin heiwa -- app runtime status --json
cargo run -q -p heiwa-shell --bin heiwa -- app update --source checkout --dry-run
```

Check the reported `cli_path`, `state_dir`, `local_app.url`, and
`local_app.reachable`. Also check update/restart hints when present. A reachable
app only proves that something is listening; it does not prove that the listener
is the code you just changed.

### 3. Avoid stale runtimes

Treat port `7474` as the installed product runtime. Do not assume it reflects
the current checkout after code edits.

For development verification, start a current checkout runtime on a temporary
alternate port:

```bash
HEIWA_EVIDENCE_DIR=/private/tmp/heiwa-operator-e2e/evidence \
HEIWA_STATE_DIR=/private/tmp/heiwa-operator-e2e/state \
HEIWA_MACHINE_AUTH_TOKEN=operator-e2e-token \
cargo run -q -p heiwa-shell --bin heiwa -- app start --port 7475 --no-open
```

`HEIWA_STATE_DIR` relocates the app shell's worker heartbeat and the state path
reported by that shell; it does not relocate every Calendar, approvals, or
other module-specific read model. Together with `HEIWA_EVIDENCE_DIR`, it keeps
the operator-stream checks below out of installed `7474` state and the durable
operator corpus. Use a disposable `HOME` with a prebuilt binary when probing
broader state-backed APIs.

Then probe that same port:

```bash
curl -fsS http://127.0.0.1:7475/status/health
curl -fsS http://127.0.0.1:7475/api/v1/session
```

If a new API endpoint returns `index.html`, the request fell through to static
SPA serving. Assume you are probing the wrong runtime, an old runtime, or an
unimplemented route until proven otherwise.

Only run `heiwa app update` when the operator explicitly wants the installed
runtime changed. `--dry-run` is the default probe. Use
`heiwa app update --source checkout` only for developer reinstall from the
current checkout.

On Apple Silicon, checkout update dry-runs report
`cargo_environment.strategy: rust_bundled_macho_linker` and the executed
`cargo install` resolves `rust-lld` from the active pinned Rust sysroot. An
explicit non-empty `CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER` remains
authoritative and is reported as `operator_override`.

Build a coherent local macOS `.app` before checkout promotion with:

```bash
npm --prefix apps/heiwa_app/desktop run tauri:build:app
```

That command builds and stages the current checkout's release `heiwa` binary,
isolates it from the case-folded `Heiwa` desktop output, disables
updater-artifact signing for the local-only bundle, and uses Rust's bundled
Mach-O linker on Apple Silicon. Release builds keep updater signing on.

### 4. Verify the authenticated operator stream

Operator HTTP and WebSocket endpoints require local runtime auth. Native Desktop
uses signed local requests: HMAC v1 binds method, numeric local port, exact
request target, SHA-256 body digest, timestamp, and nonce. Runtime accepts at
most 30 seconds of clock skew and consumes every nonce once through its bounded
replay cache. Machine bearer auth remains compatibility-only; Desktop native
transport signs HTTP and WebSocket requests and never gives a bearer to the
renderer. An unset runtime auth configuration returns `auth_not_configured`; a
missing or invalid credential returns `unauthorized`. For the isolated checkout
runtime above, use the test bearer only against `127.0.0.1:7475`:

```bash
curl -fsS \
  -H 'Authorization: Bearer operator-e2e-token' \
  -H 'Content-Type: application/json' \
  -d '{"thread_id":"default"}' \
  http://127.0.0.1:7475/api/v1/operator/threads

curl -fsS \
  -H 'Authorization: Bearer operator-e2e-token' \
  -H 'Content-Type: application/json' \
  -d '{"client_request_id":"operator-e2e-1","prompt":"reply with ready","route_policy":{"mode":"auto"}}' \
  http://127.0.0.1:7475/api/v1/operator/threads/default/turns

curl -fsS \
  -H 'Authorization: Bearer operator-e2e-token' \
  'http://127.0.0.1:7475/api/v1/operator/threads/default/events?limit=100'
```

The turn response returns `turn_id`, the stable post-user-message `cursor`, and
an encoded `stream_url`. The corresponding WebSocket request is:

```text
GET ws://127.0.0.1:7475/ws/v1/operator?thread_id=default&after=<percent-encoded-cursor>
Authorization: Bearer operator-e2e-token
```

Use a WebSocket client that can set the compatibility `Authorization` header.
For Desktop verification, launch the native wrapper with `HEIWA_APP_PORT=7475`
and the same `HEIWA_MACHINE_AUTH_TOKEN`; its Tauri bridge signs the exact GET
target below the renderer for both HTTP and WebSocket. Verify initial replay reaches
`caught_up`, a newly appended shell event arrives without refresh, and reconnect
from the last durable cursor does not duplicate an `event_id`. Heartbeats and
assistant deltas are transient and never advance the durable cursor.

Browser preview is separate: `heiwa app start --open` puts a single-use,
60-second bootstrap token in the launch URL. The runtime consumes it once and
redirects with a port-scoped HttpOnly session cookie (eight-hour TTL). Browser
code receives neither bootstrap reuse authority nor machine bearer material.

Installed mode resolves canonical secrets from environment variables first,
then from owner-private local files:

- `~/.heiwa/secrets/machine_auth_token` for `HEIWA_MACHINE_AUTH_TOKEN`
- `~/.heiwa/secrets/jwt_signing_secret` for `HEIWA_JWT_SIGNING_SECRET`

Both files must be regular, non-symlink files with mode `0600` and contain one
ASCII token. Empty, oversized, multiline, group-readable, or world-readable
files are rejected. This keeps tokens out of LaunchAgent plists while giving
the installed CLI, runtime, and native Desktop one auth source.

Cursor and restart recovery are fail-closed:

- App startup exclusively leases the configured evidence root before recovery,
  heartbeat, or API service. A second app process pointed at that same root
  exits without mutating the operator stream; isolated verification roots may
  run concurrently. The `.operator_runtime.lock` sidecar contains no identity,
  credential, or other payload.
- Every mutating `OperatorSessionService` holds a shared
  `.operator_activity.lock` lease. Recovery requires exclusive activity
  ownership, so a live CLI, REPL, loop, or compatibility writer makes app
  startup fail before heartbeat/API service and prevents false
  `RUNTIME_RESTART` interruption. Both lease sidecars remain zero-content.
- HTTP replay returns structured `invalid_cursor` for unknown versions, stream
  fingerprint mismatches, offsets beyond EOF, or offsets not on an event
  boundary. The operator client must clear its disposable projection and replay
  the thread from the beginning.
- The WebSocket sends an `invalid_cursor` frame and closes so the client can
  perform that same bounded recovery; it must not guess a replacement offset.
- On runtime restart, every nonterminal turn is durably closed with one
  `turn_interrupted` event whose reason is `RUNTIME_RESTART`. A turn with a
  pending operator cancellation closes as `OPERATOR_CANCELLED`. Open work is
  never silently resumed from process memory.
- Readers skip unknown future operator-event schema versions, count them, and
  retain known events. Never rewrite or delete durable JSONL solely because a
  newer schema is present.

### 5. Start safely

Before starting a long-running runtime, decide:

- which port it owns
- whether it is installed-product verification or checkout verification
- what command will stop it
- what files, if any, will be created for probes
- whether restart/update prompts should be shown, deferred, or ignored for this verification

Prefer `--no-open` for agent verification so the browser is not disturbed.

### 6. Use the runtime

Use the local API and cockpit against the same port you started. Keep evidence
local and concrete:

```bash
curl -fsS http://127.0.0.1:7475/status/health
curl -fsS http://127.0.0.1:7475/api/v1/runtime/snapshot
curl -fsS http://127.0.0.1:7475/api/v1/inbox
curl -fsS http://127.0.0.1:7475/api/v1/history
```

Do not fabricate cockpit rows. If the UI needs data, wire it to existing
`~/.heiwa/state` truth or add a clearly scoped read model with tests.

### 7. Stop what you started

Every agent-started runtime must be stopped before final reporting unless the
operator explicitly asks to keep it running.

Preferred stop order:

1. Send normal interrupt or SIGTERM to the exact process you started.
2. Confirm the command prints its shutdown line or the port stops responding.
3. Do not kill unrelated Heiwa processes on other ports unless the operator
   asked for that cleanup.

If sandbox policy blocks stopping a process, request escalation for the exact
PID and explain that it is the temporary runtime started for verification.

### 8. Clean as you go

Clean up temporary verification artifacts before final reporting:

- temporary JSON probe files under `/private/tmp`
- ad hoc fixture directories created by tests
- one-off logs created only for the current verification
- temporary alternate-port runtime processes

Do not delete durable runtime truth under `~/.heiwa/state`,
`~/.heiwa/sessions`, `~/.heiwa/logs`, or evidence directories unless the
operator explicitly requests it.

Before final reporting, run:

```bash
git status --porcelain=v1 -uall
```

Report remaining dirty files honestly, separating agent changes from
pre-existing or peer-agent changes.

## Codex CLI result contract

The subscription adapter wraps provider-owned `codex exec --json`. It forwards
completed assistant message items and preserves token/cache usage from
`turn.completed`. Intermediate item snapshots, reasoning, tool output, and
stderr progress are not assistant text. Stderr is discarded so an unread
diagnostic pipe cannot block the provider.

A completion event and a successful child exit are both required for success.
`turn.failed`, `error`, nonzero exit, invalid JSONL/UTF-8, and EOF without a
completion produce one normalized error instead of an empty successful answer.
Consumer cancellation stops and reaps the child even while stdout is idle or
the adapter is waiting for process exit. Supervisor cancellation retains the
shared `kill_on_drop` safeguard. Prompt arguments are separated from CLI flags.

Verify without provider credentials or inference:

```bash
cargo test -p heiwa-provider --locked --test codex_cli
```

The fixtures exercise the public adapter with disposable CLI processes and
isolated environment/state roots. They establish transport behavior, not live
account entitlement, model availability, native session continuation, or tool
effect receipts. Model selection remains with the existing routing/account
contracts.

Source: [Codex non-interactive JSONL contract](https://learn.chatgpt.com/docs/non-interactive-mode).
This applies the [latest-model guide](https://developers.openai.com/api/docs/guides/latest-model)
to the existing CLI integration; it does not claim a Responses API migration.

## Model Tier Matrix

| Lane                      | Preferred candidates when eligible | Other eligible candidates       | Notes                                                    |
| ------------------------- | ---------------------------------- | ------------------------------- | -------------------------------------------------------- |
| Routine chat/status/audit | local Ollama where sufficient      | OpenRouter, Codex, Claude Code  | Cheapest candidate above the call's quality floor        |
| Build/code                | Codex CLI, Claude Code             | Ollama coding model, OpenRouter | Provider CLIs own auth and quota semantics               |
| Research/long context     | Claude Code, Codex                 | OpenRouter                      | Route per call from live provider evidence               |
| Review/strategy           | Claude Code, Codex                 | OpenRouter                      | Use premium lanes only when the quality floor needs them |
| Sovereign work            | local Ollama tiers                 | none                            | Local-only providers; fail closed when unavailable       |
| Embeddings                | `ollama/qwen3-embedding:0.6b`      | none                            | Requires a connected local Ollama runtime                |

Gemini CLI is not a current fallback: the operator account returned
`IneligibleTierError` on 2026-07-19. Antigravity required authentication in the
same probe. Always refresh `heiwa providers`; entitlement, authentication, and
adapter discovery are separate facts.

## Verification

```bash
heiwa app update --dry-run
heiwa app runtime status --json
heiwa providers
curl -fsS http://127.0.0.1:7474/status/health
```

The installed runtime stays local. Public release readiness requires current
GitHub CI and certification, verified release assets, and a successful static
installer deployment with public install checks. A reachable localhost runtime
proves only that local surface.
