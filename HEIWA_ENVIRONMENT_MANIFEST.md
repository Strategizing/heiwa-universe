# HEIWA ENVIRONMENT MANIFEST (v2.0 - D: Migration Aligned)

## Unified System Paths (WSL/Linux)

- `HEIWA_ROOT`: `/home/devon/heiwa` (Active Repo)
- `HEIWA_HOME`: `~/.heiwa` (Symlinked to `D:/identity/.heiwa`)
- `HEIWA_IDENTITY`: `/mnt/d/identity`
- `HEIWA_MEMORY`: `/mnt/d/memory`
- `HEIWA_PROJECTS`: `/mnt/d/projects`
- `HEIWA_D_DRIVE`: `/mnt/d`
- `DEVON_OPERATOR_ROOT`: `~/.dmcgregsauce/operator`
- `CODEX_HOME`: `~/.codex` (Symlinked to `D:/identity/.codex`)
- `OPENCLAW_HOME`: `~/.openclaw` (Symlinked to `D:/identity/.openclaw`)
- `ANTIGRAVITY_HOME`: `~/.gemini` (Symlinked to `D:/identity/.gemini`)

## Toolchain Caches (D: Drive Aligned)

- `CARGO_HOME`: `/mnt/d/cache/cargo`
- `RUSTUP_HOME`: `/mnt/d/dev/rustup`
- `GOPATH`: `/mnt/d/dev/go`
- `GOCACHE`: `/mnt/d/cache/go`
- `PIP_CACHE_DIR`: `/mnt/d/cache/pip`
- `NPM_CONFIG_PREFIX`: `/mnt/d/dev/npm-global`
- `npm_config_cache`: `/mnt/d/cache/npm`
- `PNPM_HOME`: `/mnt/d/dev/pnpm-global`
- `OLLAMA_MODELS`: `/mnt/d/ai/ollama`

## Core Binaries

- `heiwax`: `/home/devon/heiwa/apps/heiwa_cli/heiwa` (V2)
- `gemini`: Managed via `D:/dev/npm-global/bin/gemini`
- `picoclaw`: `~/.local/bin/picoclaw`
- `uv`: `~/.local/bin/uv`

## Infrastructure Services (WSL/systemd)

- `heiwa-nats.service` (NATS Server with JetStream enabled)
- `heiwa-hub.service` (The Spine Orchestrator)

## Managed Tooling

- `heiwa-sdk`: Core logic (mcp, security, vault, utils)
- `heiwa-hub`: Central Hub (FastAPI, MCP Bridge, Web Static)
- `heiwax`: Unified CLI tool

## Security & Sovereignty

- `Vault`: `~/.heiwa/vault.env` (Shared via `D:/identity/.heiwa/vault.env`)
- `Redaction`: Native in SDK (`heiwa_sdk.security.redact_any`)
- `Registry`: `~/.heiwa/registry/repos/repos.json`

## Config Hierarchy

- `config/swarm/ai_router.json`: Model fallbacks and MCP servers
- `config/swarm/messaging_channels.json`: Multi-channel matrix
- `config/swarm/operator_profile.md`: Ideological core
- `config/identities/`: Persona and identity profiles
- `packages/heiwa_protocol/schemas/`: Immutable contracts

## Security Hardening

- **Master Key**: Set `HEIWA_MASTER_KEY` in your shell environment to override the default vault key.
- **Redaction**: Patterns are managed in `packages/heiwa_sdk/heiwa_sdk/security.py`. Update as new sensitive patterns emerge.
- **Access Control**: Keep `~/.heiwa/vault.env` permissions strictly limited (`chmod 600`).
- **NATS Security**: Ensure the `NATS_URL` uses `tls://` and authenticated subjects for production traffic.
