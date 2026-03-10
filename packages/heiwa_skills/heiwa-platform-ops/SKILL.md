---
name: heiwa-platform-ops
description: Heiwa-branded CLI-first platform operations and CI/CD workflow coordination across provider CLIs (`gh`, Railway, Cloudflare `wrangler`, OpenClaw, PicoClaw, and future tools). Use for profile-driven audits, deploy readiness checks, secrets/variable workflows, CI triage, runtime validation, and cross-provider operational automation.
---

# Heiwa Platform Ops

Use this as the umbrella skill for cross-provider operational work. It is the global successor to the repo-local `cli-platform-expert` seed skill.

## Default Workflow

1. Load the Heiwa project profile (use `heiwax profile validate <profile_id>` when available).
2. Probe only the CLIs needed for the task.
3. Verify auth and active target context.
4. Run read-only inspection first.
5. Run the smallest mutation needed (if approved).
6. Validate outcome and emit evidence.

## Heiwa Guardrails

- Prefer `heiwax ...` for profile-driven workflows.
- Delegate repo-specific actions to product-native commands (for example `./heiwa ops ...`) instead of duplicating logic.
- Redact secret values in all summaries and saved artifacts.
- Treat OpenClaw enablement as policy-gated pending audit/pinning.
- Keep production deploys approval-gated.

## Quick Commands

```bash
/Users/dmcgregsauce/.codex/heiwa/bin/heiwax doctor --profile heiwa-limited
/Users/dmcgregsauce/.codex/heiwa/bin/heiwax audit ci --profile heiwa-limited
/Users/dmcgregsauce/.codex/heiwa/bin/heiwax audit railway --profile heiwa-limited
/Users/dmcgregsauce/.codex/heiwa/bin/heiwax workflow run ci-audit --profile heiwa-limited --dry-run
```

## Bundled Resources

- `scripts/cli_probe.py` (copied from the seed skill)
- `references/github-gh.md`
- `references/railway-cli.md`
- `references/cloudflare-wrangler.md`
- `references/heiwa-claws.md`
- `references/workflow.md`
- `references/extending-playbooks.md`


## Verification and Pentest Sweep

Before adding new automations or enabling more write-capable workflows, run a verification sweep:

```bash
/Users/dmcgregsauce/.codex/heiwa/bin/heiwax verify redaction --profile heiwa-limited
/Users/dmcgregsauce/.codex/heiwa/bin/heiwax verify skills --profile heiwa-limited
/Users/dmcgregsauce/.codex/heiwa/bin/heiwax verify stack --profile heiwa-limited
```

Use `--full-live` on `verify stack` when you want to exercise live read-only runbooks and confirm output capture artifacts.
