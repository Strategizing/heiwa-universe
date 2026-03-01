# TOOLS.md - iClaw / Macbook Node Cheat Sheet

## 🦞 Sovereign Command Stack

- **Heiwa CLI**: `heiwa status`, `heiwa deploy`, `heiwa config`.
- **OpenClaw Control**: `openclaw status --probe`, `openclaw devices list`, `openclaw sessions list`.
- **Gateway Port**: `18789` (local).
- **Discord Console**: Channel ID `1477050626444624048`.

## 🌐 Connectivity & Mesh Channels

| Channel | Status | Note |
| :--- | :--- | :--- |
| **Discord** | **ONLINE** | `dnd` status, `partial` streaming, identify as `@Heiwa Console`. |
| **iMessage** | **SIP BLOCKED** | Native driver hits permission error on `chat.db`. |
| **BlueBubbles** | **RECOMMENDED** | Use for remote bursts if physical access is restricted. |
| **Tailscale** | **OFF** | Currently using local LAN/Mesh (can enable if needed). |

## 🏗️ Monorepo: ~/heiwa

- **Apps**: `heiwa_cli` (v2.1.0), `heiwa_hub`, `heiwa_web`.
- **Packages**: `heiwa_skills` (Cloudflare-deploy, etc.).
- **Assets**: Optimized imagery in `cloudflare-deploy` references.

## 🧠 Memory pattern

- **Daily Log**: `memory/YYYY-MM-DD.md`
- **Long-term**: `MEMORY.md` (Main session only)
- **Pulse**: `HEARTBEAT.md` (Checked every ~30 min)

---

## ⚡ Quick Fixes

- **Gateway Restart**: `openclaw gateway --force`
- **Identity Update**: `openclaw agents set-identity --agent main --from-identity`
- **Config Validate**: `openclaw config validate`
