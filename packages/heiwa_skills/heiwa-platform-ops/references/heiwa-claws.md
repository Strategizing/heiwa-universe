# Heiwa OpenClaw and PicoClaw Notes

Use this file for Heiwa-specific operational constraints and file locations.

## OpenClaw (Heiwa Policy First)

Read these files before operational work:

- `/Users/dmcgregsauce/heiwa-limited/docs/agents/openclaw/PINNED_VERSION.md`
- `/Users/dmcgregsauce/heiwa-limited/docs/agents/openclaw/SECURITY_POLICY.md`

Current local policy (at time of skill creation) indicates:

- OpenClaw is disabled pending audit/pinning
- Third-party skills are blocked

Do not enable or deploy OpenClaw changes unless the user explicitly directs that work and policy requirements are met.

## OpenClaw CLI Examples (Reference Only)

This repo's context pack includes upstream examples such as:

- `openclaw onboard --install-daemon`
- `openclaw doctor`
- `openclaw gateway --port 18789 --verbose`
- `openclaw agent --message "..." --thinking high`

Treat these as examples, not guaranteed syntax for the installed version. Confirm with local `openclaw --help`.

## PicoClaw in Heiwa

Heiwa uses PicoClaw in role-specialized worker flows (for example `scraper`, `formatter`, `checker`) rather than only as an interactive CLI.

Key code and config locations:

- `/Users/dmcgregsauce/heiwa-limited/fleets/picoclaw/worker.py`
- `/Users/dmcgregsauce/heiwa-limited/fleets/picoclaw/render_config.py`
- `/Users/dmcgregsauce/heiwa-limited/fleets/docker/Dockerfile.picoclaw`
- `/Users/dmcgregsauce/heiwa-limited/config/env/.env.picoclaw.*.railway.example`

When debugging PicoClaw execution issues, inspect:

- `PICOCLAW_BIN` resolution
- role-specific subjects/queue groups
- process timeout handling (`PICOCLAW_TIMEOUT`)
- Railway runtime logs

## Practical Debug Sequence for PicoClaw Workers

1. Confirm local/deployed env values (especially role + subject).
2. Confirm `picoclaw` binary exists in runtime or image.
3. Inspect Railway logs for timeouts, missing binary, or auth/provider errors.
4. Check worker code path in `fleets/picoclaw/worker.py` for subprocess invocation and exit-code handling.
5. Validate queue subject matches the planner/dispatcher emission path.
