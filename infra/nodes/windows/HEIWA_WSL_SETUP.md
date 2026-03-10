# Heiwa Windows/WSL Node Initialization Guide (Verified)

This guide has been successfully executed by the Antigravity Agent to initialize the "Logic Tank" Heavy Compute Node.

## ✅ Verified Environment
- **Platform:** Windows 11 + WSL2 (Ubuntu)
- **Persistence:** `systemd` enabled and managing `heiwa-worker.service`.
- **GPU Engine:** Ollama running natively in WSL with RTX 3060 passthrough.

## 📋 Status Post-Initialization
- **Worker Service:** ✅ `Active (Running)`
- **Ollama Engine:** ✅ `Online`
- **Primary Model:** 📥 `deepseek-r1:32b` (Verified download in progress).

## 🛠️ Post-Setup: Identity & Auth Handshake
To finalize the integration, the human operator must perform the following:

1.  **Open WSL terminal.**
2.  **Verify local environment:** `cat ~/heiwa/.env.worker.local`
3.  **Ensure NATS Auth:** Replace placeholders with verified credentials (`devon:noved` or your secure token).
4.  **Restart service:** `sudo systemctl restart heiwa-worker`

## 🧠 Model Quantization Note
The **RTX 3060 has 12GB VRAM**. 
- `deepseek-r1:32b` (~19GB) will spill into system RAM, resulting in slower inference.
- **Recommended for Speed:** `ollama pull deepseek-r1:14b` or `8b` to keep the model entirely on the GPU.


## High-Performance Stability (Headroom Tuning)

To ensure the Workstation remains responsive for general use while the Logic Tank is active, we have applied the following tuning:

### 1. Resource Capping (.wslconfig)
WSL is limited to 16GB RAM and 8GB Swap to prevent it from starving Windows 11.
- **File:** \C:\Users\devon\.wslconfig- **Action:** Run \wsl --shutdown\ from PowerShell to apply any changes.

### 2. GPU VRAM Strategy (12GB Managed)
We target **8GB-9.5GB** VRAM usage for LLMs, leaving 2.5GB for Windows/WSL UI.
- **Primary Models:** Mistral Nemo 12B, Qwen 2.5 Coder 14B, DeepSeek R1 14B.
- **Quantization:** Standard Olla-4bit quants are preferred as they fit perfectly within the 12GB envelope.

### 3. Monitoring
Run vidia-smi -l 1\ in a WSL terminal to monitor VRAM usage in real-time.
