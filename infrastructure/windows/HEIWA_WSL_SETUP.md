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

