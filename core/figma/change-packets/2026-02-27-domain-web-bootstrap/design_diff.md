# Heiwa Figma Sync Packet

## Packet ID
- `2026-02-27-domain-web-bootstrap`

## Architecture Changes To Reflect
- Added domain bootstrap manifest at `infrastructure/domains/heiwa-ltd.bootstrap.json`.
- Added public web domain bootstrap surface:
  - `clients/clients/web/domains.html`
  - `clients/clients/web/assets/domains.js`
  - `clients/clients/web/assets/domains.bootstrap.json`
- Added identity routing map at `core/config/identity_profiles.json`.
- Added CLI identity selector route:
  - command: `heiwa identity "<intent text>"`
  - implementation: `node/cli/scripts/ops/identity_selector.py`
- Updated operator orchestration defaults for Heiwa workspace in:
  - `~/.openclaw/openclaw.json`
  - `~/.picoclaw/config.json`
  - `~/.gemini/GEMINI.md`
  - `~/.gemini/antigravity/browserAllowlist.txt`
  - `~/.codex/config.toml`

## Runtime Evidence
- `heiwa doctor`: pass
- `node/cli/scripts/ops/heiwa_360_check.py`: `Result: READY`
- `heiwa identity "begin initializing Heiwa domains and website" --json` selects `domain-web-initializer`

## Notes
- Domain/DNS routing is initialized as a plan and bootstrap artifact; live provider cutover remains approval-gated.
