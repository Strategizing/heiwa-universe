# Railway CLI Playbook

Use this file for Railway service status, deployments, logs, variables, and Heiwa worker deployments.

## Version and Auth

Start with:

```bash
railway --version
railway whoami
railway status
```

If `railway whoami` or `railway status` fails, confirm whether the repo is linked and whether login is required.

## Context Checks

Before deploying or editing vars:

- Confirm the linked project/service/environment (`railway status`)
- Inspect local config (`railway.toml` if present)
- Verify which service you are acting on

Prefer explicit environment selection flags if your installed CLI supports them.

## Common Workflows (Examples)

Use local `railway --help` to confirm exact syntax for your CLI version.

Common patterns:

```bash
railway status
railway logs
railway up
railway run <command>
railway variables
```

Use `railway run` for commands that must see Railway-provided env vars locally (for debugging build/runtime behavior).

## Heiwa PicoClaw Railway Notes

This repo includes role-specific env templates under `/Users/dmcgregsauce/heiwa-limited/config/env/`, including:

- `.env.picoclaw.scraper.railway.example`
- `.env.picoclaw.formatter.railway.example`
- `.env.picoclaw.checker.railway.example`

Check these fields carefully when debugging worker behavior:

- `HEIWA_PICOCLAW_ROLE`
- `HEIWA_PICOCLAW_REQUEST_SUBJECT`
- `HEIWA_PICOCLAW_QUEUE_GROUP`
- `PICOCLAW_BIN`
- `PICOCLAW_TIMEOUT`

Role/subject mismatches can look like idle workers or missing responses.

## Deploy and Runtime Triage Loop

1. `railway status`
2. `railway logs`
3. Inspect local env template or deployed vars metadata
4. Apply change or deploy
5. `railway logs` again
6. Validate health endpoint / service behavior

## Common Failure Modes

- Wrong linked service/environment
- Missing Railway internal `NATS_URL` or `DATABASE_URL`
- Invalid `PICOCLAW_BIN` path or missing binary in image/runtime
- Secrets updated locally but not in target Railway environment
