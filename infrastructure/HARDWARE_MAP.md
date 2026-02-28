# Heiwa Swarm: Hardware & Compute Map

This document tracks the heterogeneous compute resources available to the Heiwa Swarm. Agents should use this map to target the correct `target_runtime` for their tasks.

## 🧠 Node 1: Mesh Brain (Cloud HQ)
*   **Platform:** Railway (Compute Hub)
*   **Specs:** 32GB vCPU + RAM
*   **Primary Role:** Orchestration, NATS Bus, Persistent PostgreSQL, Fast Planning.
*   **Target Runtime:** `railway`
*   **Preferred Models:** Gemini 2.5 Flash (via API), Gemini 1.5 Pro.

## 🍎 Node 2: Agile Edge (`macbook@heiwa-agile`)
*   **Platform:** macOS (M4 Pro)
*   **Specs:** 24GB Unified Memory
*   **Capabilities:** `agile_coding`, `workspace_interaction`, `standard_compute`
*   **Primary Role:** Agile Coding (Codex/OpenClaw), Real-time Workspace Interaction, Prototyping.
*   **Target Runtime:** `macbook@heiwa-agile`
*   **Preferred Models:** Deepseek Coder V2 (16B), Qwen 2.5 Coder (7B).

## 🪟 Node 3: Logic Tank (`wsl@heiwa-thinker`)
*   **Platform:** Windows 11 + WSL2 (Ubuntu 22.04+)
*   **Specs:** Ryzen 7 7700X, RTX 3060 (12GB VRAM), 32GB System RAM.
*   **Capabilities:** `heavy_compute`, `gpu_native`, `standard_compute`
*   **Optimizations:** 
    *   `snapd` purged (Zero loop-mount overhead).
    *   Native Ollama installation (GPU passthrough).
    *   Node.js 20.x LTS.
    *   Host-level `.wslconfig` (10 processors, GUI disabled).
*   **Primary Role:** Heavy Reasoning, Deep Technical Audit, Large Model Inference.
*   **Target Runtime:** `wsl@heiwa-thinker`
*   **Preferred Models:** Deepseek-R1 (14B/7B - *GPU Native / Lightning Fast*), Deepseek-R1 (32B - *High Precision / Spills to RAM*).

---

## 📡 Networking
All nodes are unified via **Tailscale** and communicate over the private NATS bus at the Cloud HQ IP.
*   **NATS Ingress:** `100.116.86.118:4222`
*   **Persistence:** PostgreSQL on Railway.
