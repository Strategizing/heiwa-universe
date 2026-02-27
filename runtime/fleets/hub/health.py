# fleets/hub/health.py
"""
Heiwa Core Health Server.
Railway pings /health to verify the service is alive.
"""
import asyncio
import logging
import os

from fastapi import FastAPI
import uvicorn

app = FastAPI(title="Heiwa Core", docs_url=None)
logger = logging.getLogger("heiwa.health")

@app.get("/health")
async def health():
    return {
        "status": "alive",
        "service": "heiwa-core",
        "agents": ["spine", "messenger", "openclaw", "codex"]
    }

@app.get("/")
async def root():
    return {"name": "Heiwa Limited", "version": "1.0.0", "status": "operational"}

async def _serve_health_port(port: int) -> None:
    config = uvicorn.Config(app, host="0.0.0.0", port=port, log_level="error")
    server = uvicorn.Server(config)
    logger.info("Starting health listener on port %s", port)
    await server.serve()

async def start_health_server():
    """
    Start health listeners.
    Railway health checks use PORT, while legacy domain routing may still target 8000.
    """
    ports = {int(os.environ.get("PORT", 8080))}

    extra_ports = os.environ.get("HEIWA_EXTRA_HEALTH_PORTS", "8000")
    for raw in extra_ports.split(","):
        val = raw.strip()
        if not val:
            continue
        try:
            ports.add(int(val))
        except ValueError:
            logger.warning("Ignoring invalid HEIWA_EXTRA_HEALTH_PORTS entry: %s", val)

    await asyncio.gather(*(_serve_health_port(port) for port in sorted(ports)))
