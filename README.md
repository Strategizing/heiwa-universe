# 🌐 Heiwa Universe

Canonical Monorepo for the **Heiwa Swarm**: A 24/7 autonomous heterogeneous compute mesh.

## 🏗️ Architecture
Heiwa operates as a decentralized execution mesh unified via **Tailscale** and **NATS**.

- **`/runtime`**: The Core Collective. Python-based agent logic, NATS protocol definitions, and shared SDKs.
- **`/node`**: Edge Worker logic. Specifically the `worker_manager` which bridges the local machine (Macbook/Workstation) to the Cloud HQ.
- **`/infrastructure`**: Infrastructure-as-Code. Terraform for Cloudflare, Railway configurations, and platform-specific setup guides.
- **`/clients`**: Public-facing surfaces, including the `heiwa.ltd` website.
- **`/core`**: System-wide configuration, including the `identity_profiles` router and cognitive schemas.

## 🚀 Node Quickstart
To add a new compute node to the swarm:

1.  **Provision:** Run `./node/cli/heiwa provision-node <node_id>`.
2.  **Setup:** Follow the generated instructions for your platform (macOS/Windows).
3.  **Sync:** Ensure Tailscale is active and the `NATS_URL` in `.env.worker.local` is correct.

## 🧠 Cognitive Tiers
Heiwa utilizes a tiered intelligence model:
1.  **Local (Mac/PC):** Deepseek-Coder, Qwen (Free/Instant)
2.  **Cloud API:** Gemini 2.5 Flash, Gemini 1.5 Pro
3.  **Pro/Premium:** Gemini 3 Flash, Claude Opus, Codex

---
*“Be genuinely helpful, not performatively helpful. Have opinions. Actions speak louder than filler words.”* — **SOUL.md**
