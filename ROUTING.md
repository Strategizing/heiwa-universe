# 📡 ROUTING.md - Sovereign Communication Protocol

This protocol defines how **iClaw 🦞** routes information across the Heiwa Sovereign Mesh to ensure structure, privacy, and collaborative efficiency.

## 🏛️ 1. Channel Topology

### 📱 Tactical (iMessage / BlueBubbles)
- **Use Case**: Real-time status, mobile notifications, speed pings.
- **Priority**: High urgency, low data density.
- **Tone**: Concise, immediate.

### 🔒 Personal (Discord DMs)
- **Use Case**: Sensitive context, operator-only briefings, private file exchanges.
- **Priority**: High security, focused interaction.
- **Tone**: Professional, precise.

### 🌐 Mesh Collaborative (Discord Server)
Designed for the collective "Heiwa Universe" and mesh transparency.

| Channel | Purpose | Logic |
| :--- | :--- | :--- |
| **#operator-ingress** | Default entry for tasks | Public status of current ops. |
| **#executive-briefing** | High-level outcomes | Final results and decision prompts. |
| **#ci-cd-stream** | Monorepo/Build status | Automatic logs for code/infra sync. |
| **#thought-stream** | Agent reasoning | Transparency into iClaw's "internal" plan. |
| **#central-comms** | Inter-agent bus | Coordination between iClaw and peer nodes. |
| **#swarm-telemetry** | Live metrics | CPU/RAM/Mesh health heartbeat results. |

## 🕹️ 2. Execution Logic

1. **Identify Privacy Tier**:
   - If the data contains secrets, personal user info, or private strategy -> **Discord DM**.
   - If the data is a tactical ping ("Task started", "Syncing mobile") -> **iMessage**.
   - Otherwise -> **Relevant Mesh Channel**.

2. **Structure for High-Tier High-Tier Experience**:
   - **Threading**: When executing a task in `#operator-ingress`, always **spawn a thread** named after the Task ID to keep the main channel stage clean.
   - **Rich Artifacts**: Prefer Mermaid diagrams and JSON blocks for architectural data.
   - **Escalation**: If a collaborative task requires a specific operator approval that is high-risk, ping the operator via **iMessage** with a link to the Discord thread.

3. **Context-Aware Visuals**:
   - Use `# TOPIC` for major mesh alerts.
   - Use `## SUBTOPIC` for status cards within channels.
   - Brand significant reports with `iClaw 🦞`.
