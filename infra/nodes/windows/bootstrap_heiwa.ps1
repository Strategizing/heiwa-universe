# Heiwa Swarm Bootstrapper for Windows
# Run this in PowerShell to initialize your Heavy Compute Node

$RepoUrl = "https://github.com/Strategizing/heiwa-universe.git"
$WorkDir = "$HOME/heiwa"

Write-Host "🌐 [HEIWA] Starting Windows Bootstrapper..." -ForegroundColor Cyan

# 1. Check WSL
if (!(Get-Command wsl -ErrorAction SilentlyContinue)) {
    Write-Host "❌ WSL not found. Please run 'wsl --install' and restart." -ForegroundColor Red
    exit
}

# 2. Check Tailscale
if (!(Get-Process "Tailscale" -ErrorAction SilentlyContinue)) {
    Write-Host "⚠️  Tailscale is not running. Mesh connectivity might fail." -ForegroundColor Yellow
}

# 3. Clone Repository into WSL
Write-Host "📂 Cloning Heiwa Universe into WSL..." -ForegroundColor Green
wsl bash -c "if [ ! -d ~/heiwa ]; then git clone $RepoUrl ~/heiwa; else cd ~/heiwa && git pull; fi"

# 4. Trigger Internal Setup
Write-Host "🛠️  Running internal WSL setup..." -ForegroundColor Green
wsl bash -c "cd ~/heiwa && chmod +x apps/heiwa-cli/scripts/ops/*.sh && ./apps/heiwa-cli/scripts/ops/setup_wsl_node.sh"

Write-Host "`n✨ Bootstrapping Complete." -ForegroundColor Green
Write-Host "👉 If you have a .env file from your Mac, copy it to WSL: \wsl$\Ubuntu\home\$env:USERNAME\heiwa\.env.worker.local" -ForegroundColor White
Write-Host "👉 Finally, run this in WSL: cd ~/heiwa && ./apps/heiwa-cli/scripts/ops/start_worker_stack.sh" -ForegroundColor White
