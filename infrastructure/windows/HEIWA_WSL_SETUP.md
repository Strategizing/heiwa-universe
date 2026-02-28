# Heiwa Windows/WSL Node Initialization Guide

This guide is designed for an autonomous agent (like Antigravity) or a human operator to initialize a Heiwa Heavy Compute Node on a Windows host with WSL2.

## 📋 Prerequisites
1. **Tailscale (Windows Host):** Install and log in. Ensure the machine is visible in the Heiwa Tailnet.
2. **WSL2 (Ubuntu recommended):** `wsl --install -d Ubuntu`
3. **NVIDIA Drivers:** Ensure the latest Game Ready or Studio drivers are installed on Windows for RTX 3060 support.

## 🚀 Step 1: Bootstrap from Windows (PowerShell)
Run these commands in PowerShell to prepare the environment:

```powershell
# 1. Clone the repository into WSL
wsl git clone https://github.com/Strategizing/heiwa-universe.git ~/heiwa

# 2. Grant execution permissions to setup scripts
wsl chmod +x ~/heiwa/node/cli/scripts/ops/*.sh
```

## 🛠️ Step 2: Initialize Node in WSL
Enter WSL (`wsl`) and run the specialized WSL setup script:

```bash
cd ~/heiwa
./node/cli/scripts/ops/setup_wsl_node.sh
```

## 🧠 Step 3: GPU Acceleration (Ollama)
The Workstation node uses Ollama for heavy reasoning. In WSL:

```bash
# Install Ollama for Linux
curl -fsSL https://ollama.com/install.sh | sh

# Pull the target models
ollama pull deepseek-r1:32b
ollama pull deepseek-coder-v2:16b
```

## 🔄 Step 4: Persistence
The setup script will attempt to enable `systemd` in WSL and create a Heiwa Worker service. If your WSL doesn't support systemd, you can run the worker manually in a `screen` or `tmux` session:

```bash
./node/cli/scripts/ops/start_worker_stack.sh
```

## 📡 Connectivity Check
Ensure your `.env.worker.local` has the correct `NATS_URL` pointing to the Cloud HQ's Tailscale IP (e.g., `nats://devon:noved@100.116.86.118:4222`).
