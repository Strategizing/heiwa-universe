# fleets/hub/health.py
"""
Heiwa Core Health Server.
Railway pings /health to verify the service is alive.
"""
import asyncio
import logging
import os
from pathlib import Path

from fastapi import FastAPI, Response
from fastapi.responses import FileResponse
from fastapi.staticfiles import StaticFiles
import uvicorn

app = FastAPI(title="Heiwa Core", docs_url=None)
logger = logging.getLogger("heiwa.health")
ROOT = Path(__file__).resolve().parents[2]
WEB_ROOT = ROOT / "apps" / "heiwa_web" / "clients" / "web"
ASSETS_ROOT = WEB_ROOT / "assets"

if ASSETS_ROOT.exists():
    app.mount("/assets", StaticFiles(directory=str(ASSETS_ROOT)), name="assets")


def _web_file(name: str) -> Path | None:
    candidate = WEB_ROOT / name
    return candidate if candidate.exists() else None

@app.get("/health")
async def health():
    return {
        "status": "alive",
        "service": "heiwa-core",
        "agents": ["spine", "messenger", "openclaw", "codex"]
    }


@app.head("/health")
async def health_head():
    return Response(status_code=200)

@app.get("/")
async def root():
    index = _web_file("index.html")
    if index:
        return FileResponse(index)
    return {"name": "Heiwa Limited", "version": "1.0.0", "status": "operational"}


@app.head("/")
async def root_head():
    if _web_file("index.html"):
        return Response(status_code=200)
    return Response(status_code=200)


@app.get("/status")
@app.get("/status.html")
async def status_page():
    page = _web_file("status.html")
    if page:
        return FileResponse(page)
    return {"detail": "status page unavailable"}


@app.head("/status")
@app.head("/status.html")
async def status_head():
    return Response(status_code=200 if _web_file("status.html") else 404)


@app.get("/domains")
@app.get("/domains.html")
async def domains_page():
    page = _web_file("domains.html")
    if page:
        return FileResponse(page)
    return {"detail": "domains page unavailable"}


@app.head("/domains")
@app.head("/domains.html")
async def domains_head():
    return Response(status_code=200 if _web_file("domains.html") else 404)


@app.get("/governance")
@app.get("/governance.html")
async def governance_page():
    page = _web_file("governance.html")
    if page:
        return FileResponse(page)
    return {"detail": "governance page unavailable"}


@app.head("/governance")
@app.head("/governance.html")
async def governance_head():
    return Response(status_code=200 if _web_file("governance.html") else 404)

async def _serve_health_port(port: int) -> None:
    config = uvicorn.Config(app, host="0.0.0.0", port=port, log_level="error")
    server = uvicorn.Server(config)
    logger.info("Starting health listener on port %s", port)
    try:
        await server.serve()
    except SystemExit as exc:
        logger.error("Health listener failed on port %s: %s", port, exc)

async def start_health_server():
    """
    Start health listeners.
    Railway health checks use PORT, while legacy domain routing may still target 8000.
    """
    ports = {int(os.environ.get("PORT", 8080))}

    extra_ports = os.environ.get("HEIWA_EXTRA_HEALTH_PORTS", "")
    for raw in extra_ports.split(","):
        val = raw.strip()
        if not val:
            continue
        try:
            ports.add(int(val))
        except ValueError:
            logger.warning("Ignoring invalid HEIWA_EXTRA_HEALTH_PORTS entry: %s", val)

    await asyncio.gather(*(_serve_health_port(port) for port in sorted(ports)))
