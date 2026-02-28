# HEIWA ENVIRONMENT MANIFEST

## Unified System Paths

- `HEIWA_HOME`: `/Users/dmcgregsauce/.heiwa`
- `HEIWA_ROOT`: `/Users/dmcgregsauce/heiwa`
- `HEIWA_CORE`: `/Users/dmcgregsauce/heiwa-core` (Legacy/Archive)
- `HEIWA_RUNTIME`: `/Users/dmcgregsauce/heiwa-runtime` (Legacy/Archive)
- `DEVON_OPERATOR_ROOT`: `/Users/dmcgregsauce/.dmcgregsauce/operator`
- `CODEX_HOME`: `/Users/dmcgregsauce/.codex`
- `PICOCLAW_HOME`: `/Users/dmcgregsauce/.picoclaw`
- `OPENCLAW_HOME`: `/Users/dmcgregsauce/.openclaw`
- `ANTIGRAVITY_HOME`: `/Users/dmcgregsauce/.gemini`

## Core Binaries

- `agy`: `/Users/dmcgregsauce/.antigravity/antigravity/bin/agy`
- `antigravity`: `/Users/dmcgregsauce/.antigravity/antigravity/bin/antigravity`
- `picoclaw`: `/Users/dmcgregsauce/.local/bin/picoclaw`
- `uv`: `/Users/dmcgregsauce/.local/bin/uv`
- `heiwax`: `/Users/dmcgregsauce/heiwa/apps/heiwa_cli/heiwa` (V2)

## Infrastructure Services (systemd)

- `heiwa-nats.service`
- `heiwa-worker-manager.service`

## Managed Tooling

- `heiwa-sdk`: Core logic (mcp, security, vault, utils)
- `heiwa-hub`: Central Hub (FastAPI, MCP Bridge, Web Static)
- `heiwax`: Unified CLI tool

## Security & Sovereignty

- `Vault`: `~/.heiwa/vault.env` (Authenticated via `InstanceVault`)
- `Redaction`: Native in SDK (`heiwa_sdk.security.redact_any`)
- `Registry`: `~/.heiwa/registry/repos/repos.json`

## Config Hierarchy

- `config/swarm/ai_router.json`: Model fallbacks and MCP servers
- `config/swarm/messaging_channels.json`: Multi-channel matrix
- `config/swarm/operator_profile.md`: Ideological core
- `config/identities/`: Persona and identity profiles
- `packages/heiwa_protocol/schemas/`: Immutable contracts
