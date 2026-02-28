# Heiwa Swarm: Hardware & Compute Map

This document tracks the heterogeneous compute resources available to the Heiwa Swarm. Agents should use this map to target the correct `target_runtime` for their tasks.

## 🧠 Node 1: Mesh Brain (Cloud HQ)
*   **Platform:** Railway (Compute Hub)
*   **Specs:** 32GB vCPU + RAM
*   **Primary Role:** Orchestration, NATS Bus, Persistent PostgreSQL, Fast Planning.
*   **Target Runtime:** `railway`
*   **Preferred Models:** Gemini 2.5 Flash (via API), Gemini 1.5 Pro.

## 🍎 Node 2: Agile Edge (Macbook)
*   **Platform:** macOS (M4 Pro)
*   **Specs:** 24GB Unified Memory
*   **Primary Role:** Agile Coding (Codex/OpenClaw), Real-time Workspace Interaction, Prototyping.
*   **Target Runtime:** `macbook`
*   **Preferred Models:** Deepseek Coder V2 (16B), Qwen 2.5 Coder (7B).

## 🪟 Node 3: Logic Tank (Workstation)
*   **Platform:** Windows 11 + WSL2 (Ubuntu)
*   **Specs:** Ryzen 7 7700X, RTX 3060 (12GB VRAM), 32GB System RAM.
*   **Primary Role:** Heavy Reasoning, Deep Technical Audit, Large Model Inference.
*   **Target Runtime:** `workstation`
*   **Preferred Models:** Deepseek-R1 (32B - *Note: Spills to System RAM*), Deepseek-R1 (14B/7B - *GPU Native*).

---

## 📡 Networking
All nodes are unified via **Tailscale** and communicate over the private NATS bus at the Cloud HQ IP.
*   **NATS Ingress:** `100.116.86.118:4222`
*   **Persistence:** PostgreSQL on Railway.
