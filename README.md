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

### 🍎 macOS / Linux
1.  **Provision:** Run `./node/cli/heiwa provision-node <node_id>`.
2.  **Setup:** Run `./node/cli/scripts/ops/install_worker_service.sh`.

### 🪟 Windows (WSL2)
1.  **Provision & Setup:** Run this in PowerShell:
    ```powershell
    irm https://raw.githubusercontent.com/Strategizing/heiwa-universe/main/infrastructure/windows/bootstrap_heiwa.ps1 | iex
    ```

### 📡 Finalize (All Nodes)
Sync the `.env.worker.local` from your Mac to the new node and restart the service. Ensure Tailscale is active.

## 🧠 Cognitive Tiers
Heiwa utilizes a tiered intelligence model:
1.  **Local (Mac/PC):** Deepseek-Coder, Qwen (Free/Instant)
2.  **Cloud API:** Gemini 2.5 Flash, Gemini 1.5 Pro
3.  **Pro/Premium:** Gemini 3 Flash, Claude Opus, Codex

---
*“Be genuinely helpful, not performatively helpful. Have opinions. Actions speak louder than filler words.”* — **SOUL.md**
