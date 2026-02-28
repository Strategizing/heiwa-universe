# Heiwa Verification Report (2026-02-28)

## Scope
Verification of Gemini "Genesis Cutover" claims across:
- Website/domain and edge routing (`heiwa.ltd` and subdomains)
- CLI readiness and behavior
- Actual LLM routing and provider behavior
- Agent orchestration and output surfaces

## Executive Verdict
Gemini's report was partially legitimate but materially inaccurate at the time of audit.

What was true:
- Monorepo restructure exists (`apps/*`, `packages/*`, `config/*`).
- Railway service exists and can serve Heiwa web and health routes.

What was false/broken at audit start:
- `heiwa.ltd` was not operational (Cloudflare challenge/1014 issues and later Railway fallback 404).
- Claimed "ready" CLI behavior still had noisy failure mode on NATS handshake.
- Claimed multi-provider high-tier routing was incomplete in practice (high-complexity path could return empty output under quota pressure).
- Planner schema validation path was broken and referenced a missing schema file.

## Critical Findings and Fixes Applied

### 1) Website / DNS / Edge
Initial findings:
- Cloudflare zone `security_level` was `under_attack`, forcing challenge responses.
- DNS had drift and invalid/unstable target states during cutover.
- Railway custom domains were not attached at first, causing `Application not found` for host-based routing.

Fixes applied:
- Set Cloudflare zone security level to `medium` (live API change).
- Attached custom domains to Railway service `heiwa-cloud-hq`:
  - `heiwa.ltd`, `status.heiwa.ltd`, `docs.heiwa.ltd`, `api.heiwa.ltd`, `auth.heiwa.ltd`
- Updated Cloudflare CNAMEs (proxied) to stable target:
  - `heiwa-cloud-hq-brain.up.railway.app`
- Added Railway verification TXT records (`_railway-verify*`) in Cloudflare.
- Added explicit HEAD support in hub health app so `curl -I` succeeds.

Current live status:
- `curl -I https://heiwa.ltd` -> `HTTP/2 200`
- `GET https://heiwa.ltd` -> landing HTML
- `GET https://api.heiwa.ltd/health` -> `200` JSON alive
- `GET https://auth.heiwa.ltd/health` -> `200` JSON alive

### 2) Railway Runtime and Deploy
Fixes and validations:
- Deployed multiple incremental fixes; current successful deployment:
  - `a84d9075-8bd0-4b8a-ba4b-f88bd908a425` (SUCCESS)
- Resolved startup/runtime blockers encountered in prior deployments (from previous audit pass), including missing methods and port collision class.
- Separated MCP server port from public health/web server:
  - `apps/heiwa_hub/main.py`
  - MCP now uses `HEIWA_MCP_PORT` (default `8001`) instead of colliding on `8000`.

### 3) CLI Behavior
Initial findings:
- CLI worked visually but emitted raw NATS tracebacks in offline/timeout scenarios.

Fixes applied:
- `apps/heiwa_cli/scripts/terminal_chat.py`
  - Disabled reconnect storm for offline mode (`allow_reconnect=False`, `max_reconnect_attempts=0`).
  - Added silent NATS callbacks to suppress raw stacktrace noise.
  - Retained explicit Digital Barrier token check before dispatch.

Result:
- CLI startup/operation is cleaner in offline conditions and no longer spams raw NATS tracebacks by default.

### 4) LLM Routing (Actual Behavior vs Claim)
Observed runtime keys on audited environment:
- `GEMINI_API_KEY`: set
- `ANTHROPIC_API_KEY`: missing
- `OPENAI_API_KEY`: missing

Observed provider behavior:
- `gemini_flash`: working
- `gemini_pro`: rate-limited (`429` during audit probe)
- `claude`: unavailable (no Anthropic key)
- `openai`: unavailable (no OpenAI key)

Critical bug found:
- High complexity chain originally: `gemini_pro -> claude -> openai -> ollama`
- On Railway runtime, ollama is disallowed and with missing Claude/OpenAI keys + Gemini Pro 429, high complexity could return empty output.

Fix applied:
- `apps/heiwa_hub/cognition/llm_local.py`
  - High chain changed to:
    - `gemini_pro -> gemini_flash -> claude -> openai -> ollama`
  - This prevents hard-empty responses when Pro is rate-limited.

Validated outcome:
- Low/medium/high now all return output in current env (with high gracefully degrading to Flash).

### 5) Planner / Agent Execution Path
Critical bug found:
- Planner schema path expected non-existent location (`/schemas/task_envelope_v2.schema.json`).
- Schema file itself was missing from repo.

Fixes applied:
- `apps/heiwa_hub/cognition/planner.py`
  - Schema lookup now checks:
    - `config/schemas/task_envelope_v2.schema.json`
    - legacy fallback `/schemas/...`
- Added missing schema file:
  - `config/schemas/task_envelope_v2.schema.json`

Validated outcome:
- Planner can now build and validate task envelopes without raising missing-schema errors.

## Agent and Output Reality (Current)

### Ingress and orchestration
- CLI / Discord ingress produces task envelopes.
- `SpineAgent` enforces `HEIWA_AUTH_TOKEN` barrier and dispatches step subjects.
- `ExecutorAgent` consumes execution subjects and uses `LocalLLMEngine` with complexity mapping.

### Output surfaces
- CLI output:
  - Task dispatch confirmations (`DISPATCHED`)
  - Markdown result panel on `TASK_EXEC_RESULT`
  - live telemetry footer
- Discord output (`MessengerAgent`):
  - Direct chat embed for chat intent
  - Plan embed with per-step runtime/tool
  - Approval state messages
  - status/progress/result embeds per task

### Important behavior note
For underspecified prompts, planner may prepend an "Expand request into execution brief" research step before the main intent step. This is intended but can make some commands appear to route through research first.

## Files changed in this pass
- `apps/heiwa_hub/main.py`
- `apps/heiwa_hub/health.py`
- `apps/heiwa_cli/scripts/terminal_chat.py`
- `apps/heiwa_hub/cognition/planner.py`
- `apps/heiwa_hub/cognition/llm_local.py`
- `config/schemas/task_envelope_v2.schema.json` (new)
- `infra/cloud/cloudflare/main.tf` (zone baseline codification + prior DNS/WAF alignment)

Also validated/retained prior pass fixes in:
- spine/messenger/telemetry/mcp_server/doctor/Dockerfile/web manifest updates.

## Remaining Drift / Follow-ups
1. Terraform state drift still exists for some Cloudflare resources/ruleset ownership.
2. Cloudflare Terraform uses deprecated `value` on records (works, but should migrate to `content`).
3. Anthropic/OpenAI keys are not present; high-tier quality currently relies on Gemini fallback chain.
4. Some legacy audit scripts (`heiwa_360_check.py`) still reference outdated paths and report false negatives.

## Final Readiness Status
- Public domain and API: Operational
- CLI: Operational, hardened for offline behavior
- LLM routing: Operational with graceful fallback, but premium providers require keys
- Agent pipeline: Operational for current configured routes
- Production cutover claim of "fully complete" (as originally reported): was inaccurate at audit start; now remediated to operational baseline
