# GEMINI.md — heiwa-universe

Gemini CLI is a provider-owned peer executor inside Heiwa. It owns its native
tools, system prompts, authentication, sessions, model inventory, and quotas.
Heiwa owns the local runtime, routing, and evidence around that provider surface.

## Shared Operating Contract

Use [`AGENTS.md`](AGENTS.md#operating-contract) for authorization, implementation,
review, testing, and reporting. Explicit user authorization persists across
turns; provider skills and local guidance must not add a second approval round
to already authorized work. This file supplies Gemini-specific context only.

Before runtime or architecture changes, read:

1. [`HEIWA.md`](HEIWA.md) for product and architecture truth.
2. [`AGENTS.md`](AGENTS.md) for shared working rules.
3. [`docs/local-self-operation.md`](docs/local-self-operation.md) for runtime boundaries.

When diagnosing Gemini configuration or tool policy, inspect
`.gemini/settings.json` and `.gemini/policies/heiwa-executive.toml`. Tool
availability does not expand the assigned scope. Provider-owned machine
configuration and credentials remain outside routine repository edits.

## Runtime Boundaries

- Resolve per-user roots with `crates/heiwa_config::HeiwaPaths`; never assume
  one maintainer, hardcode an owner identity, or grant privileges by alias.
- Extend the Rust service that owns the behavior. Python under
  `packages/heiwa_sdk/` is compatibility and migration code; its legacy identity,
  gateway, and tool-mesh conventions are not the Rust runtime contract.
- Resolve credentials through the existing vault and provider keychain. Keep
  secrets out of source, evidence, logs, and child environments; report the
  actual auth failure without inventing a fallback account or model.
- Verify enforcement at the execution boundary before claiming sandboxing,
  approval, or privacy protection. A declared `SandboxRequired` mode does not
  prove a sandbox backend ran the code.

## Branches and Verification

Use a short-lived experimental branch from current `dev`, then the protected PR
flow in `AGENTS.md`. An authorized publishing task includes normal push, PR,
review, and merge steps. Recheck the exact PR head, required checks, merge state,
and unresolved review threads before merging. Never commit or push directly to
`dev` or `main`, bypass protection, or overwrite another agent's changes.

Use targeted tests while iterating. Before promotion, run:

```bash
HEIWA_BRANCH_MODE=experimental bash scripts/check_agent_baseline.sh
HEIWA_BRANCH_MODE=experimental bash scripts/check_ci_local.sh
```

Use the appropriate mode on integration or post-promotion checkouts. Follow
[`docs/agent-baseline-workflow.md`](docs/agent-baseline-workflow.md) for evidence
and remote pre-flight. Report a dirty development handoff honestly; do not
commit or discard work just to satisfy a clean-tree gate.

Verify checkout runtime changes in disposable state on `7475`; installed `7474`
requires separate user authorization to change or restart. Stop temporary
processes you start unless asked to keep them running.
