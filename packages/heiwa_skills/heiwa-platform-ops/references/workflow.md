# CLI and CI Workflow

Use this file for provider-agnostic execution patterns.

## Intake Template

Capture these fields before running commands:

- Provider/tool (`gh`, `wrangler`, `railway`, `openclaw`, `picoclaw`, other)
- Target project/service/app
- Target environment (dev, preview, staging, prod)
- Requested outcome (inspect, deploy, rollback, set secret, debug CI, smoke test)
- Risk level (read-only vs mutating)

If any field is missing for a mutating request, ask before proceeding.

## Commanding Pattern

Use this order:

1. `which` / `--version`
2. `--help` (root and relevant subcommand)
3. Auth/context checks
4. Read-only state/logs
5. Mutating command
6. Validation command

Prefer exact, single-purpose commands over long pipes.

## CI Triage Pattern (Any Stack)

Use this sequence when the user says "CI is failing":

1. Identify the CI system (GitHub Actions, provider build logs, both).
2. Pull the latest failed run/job logs.
3. Classify failure:
   - syntax/build
   - test
   - dependency/install
   - secret/config
   - deployment/runtime
4. Reproduce locally only if needed and feasible.
5. Apply the smallest fix.
6. Re-run or re-trigger only the relevant pipeline.
7. Verify post-fix artifacts/deploy health.

If the failure is in deploy/runtime, switch to the provider CLI and inspect service logs/status immediately after CI logs.

## Safe Defaults

- Prefer machine-readable output (`--json`) when available.
- Prefer explicit environment/project flags over implicit context.
- Avoid inline secrets in commands.
- Avoid interactive login/install flows unless requested.
- Use short timeouts for probes and auth checks.
- Confirm before deletes, force flags, or production secret rotations.

## Evidence to Return

Include:

- What CLI/version was used
- What context/account/project was confirmed
- What command(s) ran
- What changed (or did not)
- What validation proved success/failure

This makes later debugging and audits faster.
