# Heiwa Recovery + Refactor Plan (Verified Feb 28, 2026)

## Summary
Restore service reliability first, then refactor ingress/orchestration so CLI dispatch, Spine planning, auth, and NATS transport are deterministic and observable.

## Latest Updates (Verified Now)
1. Railway `heiwa-cloud-hq` latest deployment failed on **Feb 28, 2026 09:20 PST** (`f4e7b5c5-2cac-4a00-a90b-b90b5e1e05ce`) due `NameError: spine is not defined`.
2. The active successful deployment is older (`35f5f868-982c-406a-b595-4ea5e4ffa608`, **Feb 28, 2026 07:38 PST**) and logs show:
- `Invalid token from sender None`
- `No steps to dispatch for task ...`
- `backend -> client close connection ... 127.0.0.1:4222`
3. Current repo has a boot regression in [apps/heiwa_hub/main.py](/Users/dmcgregsauce/heiwa/apps/heiwa_hub/main.py:60) where `spine/executor/telemetry` are referenced but never instantiated.
4. Current repo does include improved Spine auto-planning + fallback logic in [spine.py](/Users/dmcgregsauce/heiwa/apps/heiwa_hub/agents/spine.py:77) and token embedding in [terminal_chat.py](/Users/dmcgregsauce/heiwa/apps/heiwa_cli/scripts/terminal_chat.py:238), but that code is not live because the latest deployment crashes.
5. `heiwax verify stack --profile heiwa-limited` currently fails a workflow-contract check: unknown `command_ref "railway_services"` (repo contract path mismatch).

## Legitimacy Check of Gemini Report
- `Protocol Schism`: **Partially true**. One-shot CLI currently routes through TUI stack, which is heavy and fragile for command execution.
- `Digital Barrier Paradox`: **Mostly true**. Auth failures are real, but not primarily from double-wrap stripping in current code; they come from mixed producers with inconsistent envelope/auth fields.
- `Silent Planning Failure`: **True in live deployment**. Active version blocks on empty `steps` and does not auto-plan raw text.
- `NATS Island/Bridge Fragility`: **True**. Recurrent bridge-side disconnects are present in live logs.

## Refactor Plan

### Phase 0: Immediate Stabilization (Hotfix + Redeploy)
1. Fix boot regression by reinstating agent instantiation in [main.py](/Users/dmcgregsauce/heiwa/apps/heiwa_hub/main.py).
2. Add startup guard: fail-fast with explicit log if any core agent object is undefined before scheduling tasks.
3. Redeploy `heiwa-cloud-hq` to `brain`.
4. Verify:
- `/health` returns 200.
- Logs contain `Spine Active` + `Executor Active`.
- No `NameError` restart loop.

### Phase 1: Single Canonical Ingress Envelope
1. Create shared envelope adapter module in hub runtime (`apps/heiwa_hub`) for:
- `extract_auth_token(envelope)`
- `extract_payload(envelope)`
- `normalize_sender(envelope)`
2. Make Spine consume only normalized envelope shape internally.
3. Update all local producers to emit the same shape:
- [terminal_chat.py](/Users/dmcgregsauce/heiwa/apps/heiwa_cli/scripts/terminal_chat.py)
- [send_task.py](/Users/dmcgregsauce/heiwa/apps/heiwa_cli/scripts/send_task.py)
- [sota_verify.py](/Users/dmcgregsauce/heiwa/apps/heiwa_cli/scripts/ops/sota_verify.py)
- [smoke_test.py](/Users/dmcgregsauce/heiwa/apps/heiwa_hub/actions/smoke_test.py)
4. Backward compatibility: Spine still accepts legacy envelopes for one release cycle, but emits explicit reject reasons to `TASK_STATUS` instead of silent drop.

### Phase 2: Split CLI Runtime (`chat` vs `dispatch`)
1. Keep `heiwa chat` as long-lived interactive TUI only.
2. Add `heiwa dispatch "<prompt>"` one-shot path:
- New lightweight script `apps/heiwa_cli/scripts/dispatch_once.py`.
- Performs single NATS request/reply and exits.
- 5-second timeout returns deterministic network error.
3. CLI default behavior:
- `heiwa <text>` maps to `dispatch`.
- `heiwa chat` required for TUI mode.
4. Add explicit ACK contract in Spine reply payload:
- `accepted`
- `task_id`
- `reason` (if rejected)

### Phase 3: Deterministic Planning + Execution Guarantees
1. In Spine, enforce invariant: every accepted task yields at least one executable step.
2. Planning path:
- If `steps` present, validate and dispatch.
- Else if `raw_text` present, run planner.
- If planner fails/empty, synthesize fallback `TASK_EXEC` step.
3. Remove/replace ambiguous `No steps to dispatch` paths with structured status codes:
- `BLOCKED_AUTH`
- `BLOCKED_NO_CONTENT`
- `DISPATCHED_FALLBACK`
- `DISPATCHED_PLAN`

### Phase 4: NATS Topology Hardening
1. Make Cloud HQ direct-to-Railway NATS the default transport.
2. Gate local bridge/leaf mode behind explicit env flag (default off in cloud).
3. Add `heiwa-handshake` command that validates full vertical path:
- Local env + token presence
- Reachability to configured NATS
- Spine ACK round-trip
- `TASK_EXEC_RESULT` receipt window
4. Block `heiwa dispatch` if handshake health is red (clear error + suggested fix).

### Phase 5: Auth Barrier Hardening (Without Breaking Current Security)
1. Keep payload token check as baseline.
2. Add optional transport-auth mode flag for future NKey migration:
- `HEIWA_AUTH_MODE=payload_token` (default)
- `HEIWA_AUTH_MODE=nats_transport`
3. In payload-token mode, all reject events are observable with sender/task metadata.
4. Do not remove token checks until NATS auth is fully validated in both local and Railway paths.

### Phase 6: Ops/Profile Contract Repair
1. Add/align repo contract instance file expected by heiwax runbooks at:
- `config/heiwa/cloud_hq_repo_contract_v1.json`
2. Ensure `command_refs` includes `railway_services` and all runbook references used by `railway-deploy-readiness`.
3. Re-run:
- `heiwax verify redaction --profile heiwa-limited`
- `heiwax verify skills --profile heiwa-limited`
- `heiwax verify stack --profile heiwa-limited`

### Phase 7: Architecture Sync Packet (Required by Heiwa policy)
1. After Phases 1-4, generate/update Figma packet under:
- `/Users/dmcgregsauce/.codex/heiwa/figma/change-packets/`
2. Include:
- Updated ingress contract
- Runtime data flow
- Auth mode matrix
- Bridge vs direct NATS topology
3. Keep human-in-the-loop review before visual architecture update.

## Public API / Interface Changes
1. CLI:
- New: `heiwa dispatch "<prompt>"`
- Existing `heiwa chat` remains
- Default `heiwa <text>` now dispatches (non-TUI path)
2. Protocol:
- Add request/reply subject for sync ACK (for dispatch path).
- Structured ACK/REJECT payload fields: `accepted`, `task_id`, `reason`, `status_code`.
3. Status semantics:
- Replace ambiguous free-text failure with machine-parseable status codes listed above.

## Test Cases and Scenarios
1. Boot regression test:
- Hub startup succeeds and all core agents instantiate.
2. Auth tests:
- Valid token accepted.
- Missing/invalid token rejected with explicit status.
3. Dispatch tests:
- `heiwa dispatch` receives ACK under 5s on healthy path.
- Timeout error under network failure.
4. Planner tests:
- Raw text with no steps still dispatches via auto-plan or fallback.
- Planner exception still dispatches fallback.
5. Topology tests:
- Direct NATS mode stable.
- Bridge mode explicitly enabled and verified.
6. End-to-end smoke:
- `dispatch -> spine ACK -> executor result -> CLI render` within SLA.

## Assumptions and Defaults
1. Priority is production stability before deeper architectural migration.
2. Default auth mode remains `payload_token` until transport auth is proven.
3. Default cloud NATS mode is direct (bridge disabled unless explicitly required).
4. CLI UX priority is deterministic command execution over TUI-first behavior.
5. Profile target is `heiwa-limited` and environment focus is Railway `brain`.
