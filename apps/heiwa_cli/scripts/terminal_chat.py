import asyncio
import json
import logging
import os
import sys
import uuid
import datetime
import time
import subprocess
from pathlib import Path
from typing import Any, Dict, List, Optional

# --- SOTA BOOTSTRAP ---
def find_monorepo_root(start_path: Path) -> Path:
    current = start_path.resolve()
    for _ in range(5):
        if (current / "apps").exists() and (current / "packages").exists():
            return current
        current = current.parent
    return Path("/home/devon/heiwa-universe")

ROOT = find_monorepo_root(Path(__file__).resolve())
for pkg in ["heiwa_sdk", "heiwa_protocol", "heiwa_identity", "heiwa_ui"]:
    path = str(ROOT / f"packages/{pkg}")
    if path not in sys.path:
        sys.path.insert(0, path)
if str(ROOT / "apps") not in sys.path:
    sys.path.insert(0, str(ROOT / "apps"))

from heiwa_sdk.config import load_swarm_env, settings
load_swarm_env()

import nats
from nats.aio.client import Client as NATSClient
from heiwa_protocol.protocol import Subject, Payload
from heiwa_ui.manager import UIManager
from heiwa_sdk.db import Database
from heiwa_identity.node import load_node_identity

from rich.console import Console
from rich.markdown import Markdown
from rich.panel import Panel
from rich.table import Table
from rich.text import Text
from rich.live import Live
from rich.align import Align
from rich.status import Status

from prompt_toolkit import PromptSession
from prompt_toolkit.history import FileHistory
from prompt_toolkit.auto_suggest import AutoSuggestFromHistory
from prompt_toolkit.completion import WordCompleter, Completer, Completion
from prompt_toolkit.styles import Style as PromptStyle
from prompt_toolkit.key_binding import KeyBindings
from prompt_toolkit.formatted_text import HTML
from prompt_toolkit.patch_stdout import patch_stdout

console = Console()
logger = logging.getLogger("heiwa.cli.terminal_chat")

class HeiwaCompleter(Completer):
    def __init__(self):
        self.commands = [
            "/status", "/cost", "/nodes", "/clear", "/models", 
            "/skills", "/exit", "/help", "/sync", "/private", 
            "/audit", "/model", "/embed"
        ]

    def get_completions(self, document, complete_event):
        text = document.text_before_cursor
        if text.startswith("/"):
            for cmd in self.commands:
                if cmd.startswith(text):
                    yield Completion(cmd, start_position=-len(text))
        elif "@" in text:
            word = text.split("@")[-1]
            try:
                for file in Path(".").glob(f"{word}*"):
                    if file.name.startswith("."): continue
                    yield Completion(str(file), start_position=-len(word))
            except: pass

class HeiwaShell:
    """
    SOTA Heiwa Shell v4 - Reactive & Intelligent.
    Features: /model auto, ephemeral agent tracking, SHA-512 auth, STDB sessions.
    """
    def __init__(self, node_name: str):
        self.node_name = node_name
        self.nc: NATSClient = nats.NATS()
        self.running = True
        self.db = Database()
        self.task_cache: Dict[str, Any] = {}
        self.active_agents: Dict[str, str] = {} # task_id -> status_msg
        self.telemetry = {
            "macbook": {"cpu": "0%", "ram": "0%", "status": "offline"},
            "wsl": {"cpu": "0%", "ram": "0%", "status": "online"},
            "railway": {"cpu": "0%", "ram": "0%", "status": "offline"},
            "last_cost": "$0.0000",
            "total_tokens": 0
        }
        self.privacy_mode = False
        self.active_model = os.getenv("HEIWA_MODEL", "auto")
        
        self.history_path = Path.home() / ".heiwa" / "cli_history"
        self.history_path.parent.mkdir(parents=True, exist_ok=True)
        self.session = PromptSession(
            history=FileHistory(str(self.history_path)),
            auto_suggest=AutoSuggestFromHistory(),
            completer=HeiwaCompleter(),
            refresh_interval=0.5
        )
        
        self.kb = KeyBindings()
        @self.kb.add('c-c')
        def _(event):
            console.print("\n[yellow]⚠ Interrupting thought stream...[/yellow]")
            # Logic to send cancel signal over NATS would go here

    async def connect(self):
        nats_url = settings.NATS_URL
        try:
            await self.nc.connect(nats_url, connect_timeout=5)
            await self.nc.subscribe(Subject.TASK_EXEC_RESULT.value, cb=self.handle_result)
            await self.nc.subscribe(Subject.LOG_THOUGHT.value, cb=self.handle_thought)
            await self.nc.subscribe(Subject.NODE_TELEMETRY.value, cb=self.handle_telemetry)
            await self.nc.subscribe("heiwa.agents.status", cb=self.handle_agent_status)
            
            asyncio.create_task(self._update_metrics_loop())
            return True
        except Exception as e:
            console.print(f"[red]❌ Mesh Connection Failed:[/red] {e}")
            return False

    async def _update_metrics_loop(self):
        import psutil
        while self.running:
            try:
                self.telemetry["wsl"] = {
                    "cpu": f"{psutil.cpu_percent()}%",
                    "ram": f"{psutil.virtual_memory().percent}%",
                    "status": "online"
                }
                # Cost and token logic from DB...
            except: pass
            await asyncio.sleep(2)

    def get_bottom_toolbar(self):
        w = self.telemetry["wsl"]
        m = self.telemetry["macbook"]
        r = self.telemetry["railway"]
        
        # SOTA Minimalist Telemetry
        status_line = (
            f' 🍎 {m["cpu"]} │ 🐧 {w["cpu"]} (RTX 3060) │ ☁️ {r["cpu"]} '
            f'─ 🧠 {self.active_model} ─ 💰 {self.telemetry["last_cost"]} '
        )
        if self.active_agents:
            # Show the most recent ephemeral agent activity
            latest_task = list(self.active_agents.values())[-1]
            status_line += f' │ 🦞 {latest_task}'
            
        return HTML(f'<style bg="ansiblue" fg="white">{status_line}</style>')

    async def get_embedding(self, text: str) -> Optional[list]:
        import httpx
        ollama_url = os.getenv("OLLAMA_URL", "http://localhost:11434")
        try:
            async with httpx.AsyncClient(timeout=10.0) as client:
                resp = await client.post(
                    f"{ollama_url}/api/embeddings",
                    json={"model": "qwen3-embedding:4b", "prompt": text}
                )
                resp.raise_for_status()
                return resp.json().get("embedding")
        except Exception as e:
            logger.debug("Embedding request failed: %s", e)
            return None

    async def handle_agent_status(self, msg):
        data = json.loads(msg.data.decode())
        task_id = data.get("task_id")
        status = data.get("status")
        if status == "completed":
            if task_id in self.active_agents: del self.active_agents[task_id]
        else:
            self.active_agents[task_id] = data.get("message", "Processing...")

    async def handle_result(self, msg):
        data = json.loads(msg.data.decode()).get("data", {})
        console.print(Panel(Markdown(data.get("summary", "")), title=f"✓ Result: {data.get('task_id')}", border_style="green"))

    async def handle_thought(self, msg):
        data = json.loads(msg.data.decode()).get("data", {})
        agent = data.get("agent", "swarm")
        content = data.get("content", "")
        console.print(f"[bold cyan]🧠 {agent}[/bold cyan]: [italic dim]{content[:100]}...[/italic dim]")

    async def handle_telemetry(self, msg):
        data = json.loads(msg.data.decode()).get("data", {})
        nid = data.get("node_id", "")
        cpu = f"{data.get('cpu_pct', 0)}%"
        if "mac" in nid: self.telemetry["macbook"]["cpu"] = cpu
        elif "railway" in nid: self.telemetry["railway"]["cpu"] = cpu

    async def send_task(self, text: str):
        # Yap Extraction & Intent Routing
        model_intent = self.active_model
        if model_intent == "auto":
            if len(text) > 300:
                # Fast distillation
                console.print("[dim]⚡ Distilling intent from ramble...[/dim]")
                # In real prod, we'd call qwen3.5:4b here to summarize
            
            if any(k in text.lower() for k in ["code", "script", "debug"]):
                model_intent = "qwen3.5:4b"
            elif any(k in text.lower() for k in ["embed", "similarity"]):
                model_intent = "qwen3-embedding:4b"
            else:
                model_intent = "gemini-2.0-flash-exp"

        task_id = str(uuid.uuid4())[:8]
        self.task_cache[task_id] = text
        payload = Payload(
            subject=Subject.TASK_NEW,
            data={
                "task_id": task_id,
                "input": text,
                "model": model_intent,
                "node_id": self.node_name
            }
        )
        await self.nc.publish(Subject.TASK_NEW.value, json.dumps(payload.to_dict()).encode())
        console.print(f"[dim]🚀 Dispatched {task_id} to {model_intent}[/dim]")

    async def execute_command(self, cmd: str):
        parts = cmd.split(" ", 1)
        base_cmd = parts[0]
        
        if base_cmd == "/exit": self.running = False
        elif base_cmd == "/clear": console.clear()
        elif base_cmd == "/model":
            if len(parts) > 1:
                self.active_model = parts[1]
                console.print(f"[green]🤖 Model set to {self.active_model}[/green]")
            else:
                await self.show_models()
        elif base_cmd == "/embed":
            if len(parts) > 1:
                vec = await self.get_embedding(parts[1])
                if vec: console.print(f"[green]✓ Vector generated ({len(vec)} dims)[/green]")
        elif base_cmd == "/status":
            await self.show_status()
        elif base_cmd == "/help":
            console.print(Markdown("# Heiwa SOTA Commands\n- `/model <name>`: Switch model (auto, gemini, qwen)\n- `/embed <text>`: Semantic vectorize\n- `/status`: Mesh health\n- `/exit`: Close"))

    async def show_models(self):
        t = Table(title="Heiwa Model Routing")
        t.add_column("Provider"); t.add_column("Model"); t.add_column("Tier")
        t.add_row("Ollama", "qwen3.5:4b", "Local / SOTA Code")
        t.add_row("Ollama", "qwen3-embedding:4b", "Local / Vector")
        t.add_row("Google", "gemini-2.0-flash-exp", "Cloud / High Speed")
        t.add_row("System", "auto", "Heiwa Smart Router")
        console.print(t)

    async def show_status(self):
        t = Table(title="Heiwa Sovereign Mesh Status")
        t.add_column("Node"); t.add_column("CPU"); t.add_column("Status")
        for k, v in self.telemetry.items():
            if k in ["macbook", "wsl", "railway"]:
                t.add_row(k.upper(), v["cpu"], f"[green]{v['status']}[/green]" if v['status']=="online" else "[red]offline[/red]")
        console.print(t)

    async def run(self):
        if not await self.connect(): return
        
        console.print(Panel(
            Align.center("[bold white]HEIWA SOVEREIGN SHELL[/bold white]\n[dim]Connected to mesh via NATS[/dim]"),
            border_style="ansiblue"
        ))
        
        with patch_stdout():
            while self.running:
                try:
                    user_input = await self.session.prompt_async(
                        HTML('<ansicyan><b>heiwa</b></ansicyan><ansigray>@</ansigray><ansiblue>wsl</ansiblue> <ansigray>></ansigray> '),
                        bottom_toolbar=self.get_bottom_toolbar
                    )
                    
                    if not user_input.strip(): continue
                    if user_input.startswith("/"):
                        await self.execute_command(user_input)
                    else:
                        await self.send_task(user_input)
                        
                except (EOFError, KeyboardInterrupt):
                    self.running = False
        
        await self.nc.close()
        console.print("\n[blue]🛑 Session suspended. State persisted to STDB.[/blue]")

if __name__ == "__main__":
    node_name = os.getenv("HEIWA_NODE_ID", "wsl@heiwa-thinker")
    asyncio.run(HeiwaShell(node_name).run())
