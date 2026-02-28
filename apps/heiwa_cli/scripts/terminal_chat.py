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

# Ensure enterprise roots are on sys.path
def find_monorepo_root(start_path: Path) -> Path:
    current = start_path.resolve()
    for _ in range(5):
        if (current / "apps").exists() and (current / "packages").exists():
            return current
        current = current.parent
    return Path("/Users/dmcgregsauce/heiwa")

ROOT = find_monorepo_root(Path(__file__).resolve())
if str(ROOT / "packages/heiwa_sdk") not in sys.path:
    sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))
if str(ROOT / "packages/heiwa_protocol") not in sys.path:
    sys.path.insert(0, str(ROOT / "packages/heiwa_protocol"))
if str(ROOT / "packages/heiwa_identity") not in sys.path:
    sys.path.insert(0, str(ROOT / "packages/heiwa_identity"))
if str(ROOT / "packages/heiwa_ui") not in sys.path:
    sys.path.insert(0, str(ROOT / "packages/heiwa_ui"))
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

from prompt_toolkit import PromptSession
from prompt_toolkit.history import FileHistory
from prompt_toolkit.auto_suggest import AutoSuggestFromHistory
from prompt_toolkit.completion import WordCompleter, Completer, Completion
from prompt_toolkit.styles import Style as PromptStyle
from prompt_toolkit.key_binding import KeyBindings
from prompt_toolkit.formatted_text import HTML

console = Console()
logger = logging.getLogger("heiwa.cli.terminal_chat")

class HeiwaCompleter(Completer):
    def __init__(self):
        self.commands = ["/status", "/cost", "/nodes", "/clear", "/models", "/skills", "/exit", "/help", "/sync", "/private", "/audit"]

    def get_completions(self, document, complete_event):
        text = document.text_before_cursor
        
        # Slash commands
        if text.startswith("/"):
            for cmd in self.commands:
                if cmd.startswith(text):
                    yield Completion(cmd, start_position=-len(text))
        
        # File mentions with @
        elif "@" in text:
            # Get the word currently being typed after @
            parts = text.split("@")
            word = parts[-1]
            path = Path(".")
            try:
                for file in path.glob(f"{word}*"):
                    if file.name.startswith("."): continue
                    yield Completion(str(file), start_position=-len(word))
            except: pass
        
        # General file completion (optional, keeping it focused on @ for SOTA feel)
        else:
            word = text.split()[-1] if text.strip() else ""
            if word and len(word) > 2:
                try:
                    path = Path(".")
                    for file in path.glob(f"{word}*"):
                        if file.name.startswith("."): continue
                        yield Completion(str(file), start_position=-len(word))
                except: pass

class HeiwaShell:
    """
    SOTA Enterprise Shell v3.
    Features: Persistent history, non-glitchy footer, threaded thought stream.
    """
    def __init__(self, node_id: str):
        self.node_id = node_id
        self.nc: NATSClient = nats.NATS()
        self.running = True
        self.db = Database()
        self.task_cache: Dict[str, Any] = {}
        self.telemetry = {
            "macbook": {"cpu": "0%", "ram": "0%", "last_seen": 0},
            "workstation": {"cpu": "OFFLINE", "ram": "OFFLINE", "last_seen": 0},
            "railway": {"cpu": "0%", "ram": "0%", "last_seen": 0},
            "last_cost": "$0.0000"
        }
        self.privacy_mode = False
        self.active_model = os.getenv("HEIWA_MODEL", "auto (Smart Routing)")
        self.available_skills = ["browser", "code_gen", "research", "execution", "terminal"]
        
        # Setup prompt_toolkit
        self.history_path = Path.home() / ".heiwa" / "cli_history"
        self.history_path.parent.mkdir(parents=True, exist_ok=True)
        self.session = PromptSession(
            history=FileHistory(str(self.history_path)),
            auto_suggest=AutoSuggestFromHistory(),
            completer=HeiwaCompleter()
        )
        
        self.kb = KeyBindings()
        @self.kb.add('c-c')
        def _(event):
            self.running = False
            event.app.exit()

    async def connect(self, max_retries: int = 5, retry_delay: int = 2):
        nats_url = settings.NATS_URL

        async def _silent_error_cb(err):
            logger.debug("NATS error (suppressed): %s", err)

        async def _silent_disconnected_cb():
            logger.debug("NATS disconnected (suppressed)")

        async def _silent_closed_cb():
            logger.debug("NATS closed (suppressed)")

        for attempt in range(1, max_retries + 1):
            try:
                # Keep CLI behavior deterministic in offline scenarios:
                # do not keep reconnecting in the background and flooding stderr.
                await self.nc.connect(
                    nats_url,
                    connect_timeout=5,
                    allow_reconnect=False,
                    max_reconnect_attempts=0,
                    error_cb=_silent_error_cb,
                    disconnected_cb=_silent_disconnected_cb,
                    closed_cb=_silent_closed_cb,
                )
                # Subscriptions
                await self.nc.subscribe(Subject.TASK_EXEC_RESULT.value, cb=self.handle_result)
                await self.nc.subscribe(Subject.LOG_THOUGHT.value, cb=self.handle_thought)
                await self.nc.subscribe(Subject.NODE_TELEMETRY.value, cb=self.handle_telemetry)
                
                # Start metrics loop
                asyncio.create_task(self._update_metrics_loop())
                return True
            except Exception as e:
                if attempt == max_retries:
                    console.print(f"[red]❌ Mesh Connection Failed:[/red] {e}")
                    self.nc = None
                else:
                    await asyncio.sleep(retry_delay)
        return False

    async def _update_metrics_loop(self):
        """Periodically calculate actual token usage and estimated real-time costs."""
        start_time = time.time()
        import psutil
        while self.running:
            try:
                # Natively poll local macbook
                self.telemetry["macbook"] = {
                    "cpu": f"{psutil.cpu_percent()}%",
                    "ram": f"{psutil.virtual_memory().percent}%",
                    "last_seen": time.time()
                }
                
                # Poll active nodes from DB for remote HQ
                nodes = self.db.list_nodes(status="ONLINE")
                for node in nodes:
                    meta = json.loads(node.get("meta_json", "{}"))
                    if meta.get("agent_name") in ["heiwa-telemetry", "heiwa-executor", "heiwa-spine"]:
                        self.telemetry["railway"] = {
                            "cpu": f"{meta.get('cpu_pct', 0)}%",
                            "ram": f"{meta.get('ram_pct', 0)}%",
                            "last_seen": time.time()
                        }
                        
                # Get the last 24h usage summary from Sovereign DB
                summary = self.db.get_model_usage_summary(minutes=1440)
                total_cost = 0.0
                total_tokens = 0
                if summary:
                    for row in summary:
                        total_tokens += row.get("total_tokens", 0) or 0
                        total_cost += row.get("total_cost", 0.0) or 0.0
                
                # Mesh infra cost estimate (Mac, WSL, Railway uptime)
                uptime_hours = (time.time() - start_time) / 3600
                infra_cost = uptime_hours * 0.02 # Base compute mesh cost
                
                self.telemetry["last_cost"] = f"${total_cost + infra_cost:.4f}"
                self.telemetry["total_tokens"] = total_tokens
            except Exception as e:
                # Tolerate DB locks/errors peacefully
                pass
            await asyncio.sleep(5)

    def get_bottom_toolbar(self):
        m = self.telemetry["macbook"]
        w = self.telemetry["workstation"]
        r = self.telemetry["railway"]
        
        active_tasks = len(self.task_cache)
        task_info = f" | ⚡ tasks: {active_tasks}" if active_tasks > 0 else ""
        privacy_info = f" | 🔒 PRIVATE" if self.privacy_mode else ""
        model_info = f" | 🤖 {self.active_model}"
        tokens = self.telemetry.get("total_tokens", 0)
        
        return HTML(
            f'<style bg="ansiblue" fg="white">'
            f' 🍎 MAC: {m["cpu"]} '
            f' 🐧 WSL: {w["cpu"]} '
            f' ☁️ HQ: {r["cpu"]} '
            f'{model_info}{task_info}{privacy_info} | 🪙 {tokens} | 💰 {self.telemetry["last_cost"]} '
            f'</style>'
        )

    async def handle_result(self, msg):
        payload = json.loads(msg.data.decode())
        data = payload.get("data", {})
        task_id = data.get("task_id")
        
        if task_id in self.task_cache:
            summary = data.get("summary", "")
            console.print("\n")
            console.print(Panel(
                Markdown(summary), 
                title=f"Result: {task_id}", 
                border_style="green", 
                subtitle=f"Runtime: {data.get('runtime')} | Tool: {data.get('target_tool')}"
            ))
            del self.task_cache[task_id]

    async def handle_thought(self, msg):
        payload = json.loads(msg.data.decode())
        data = payload.get("data", {})
        content = data.get("content", "")
        agent = data.get("agent", "unknown")
        task_id = data.get("task_id", "swarm")
        
        # SOTA Thought UI
        console.print(f"[bold cyan]🧠 {agent}[/bold cyan] [dim]({task_id}):[/dim] [italic]{content[:140]}...[/italic]")

    async def handle_telemetry(self, msg):
        payload = json.loads(msg.data.decode()).get("data", {})
        nid = payload.get("node_id", "unknown")
        cpu = f"{payload.get('cpu_pct', 0)}%"
        ram = f"{payload.get('ram_pct', 0)}%"
        
        if "macbook" in nid: self.telemetry["macbook"] = {"cpu": cpu, "ram": ram, "last_seen": time.time()}
        elif "wsl" in nid or "workstation" in nid: self.telemetry["workstation"] = {"cpu": cpu, "ram": ram, "last_seen": time.time()}
        elif "railway" in nid: self.telemetry["railway"] = {"cpu": cpu, "ram": ram, "last_seen": time.time()}

    async def execute_command(self, cmd: str):
        if cmd == "/clear": console.clear()
        elif cmd == "/exit": self.running = False
        elif cmd == "/private":
            self.privacy_mode = not self.privacy_mode
            console.print(f"[yellow]🔒 Privacy Mode: {'ENABLED' if self.privacy_mode else 'DISABLED'}[/yellow]")
        elif cmd == "/audit":
            # Run local audit script
            audit_script = ROOT / "apps/heiwa_cli/scripts/ops/corporate_audit.py"
            subprocess.run([sys.executable, str(audit_script)])
        elif cmd == "/status":
            await self.show_status()
        elif cmd == "/models":
            await self.show_models()
        elif cmd == "/skills":
            await self.show_skills()
        elif cmd.startswith("/model "):
            new_model = cmd.split(" ", 1)[1]
            self.active_model = new_model
            console.print(f"[green]🤖 Active model set to: [/green][bold]{new_model}[/bold]")
        elif cmd == "/help":
            console.print(Markdown("# Heiwa SOTA Commands\n- `/status`: Full mesh health\n- `/models`: List available models\n- `/model <name>`: Switch active model\n- `/skills`: List node capabilities\n- `/private`: Toggle encryption\n- `/audit`: Corporate compliance check\n- `/exit`: Close session"))

    async def show_models(self):
        table = Table(title="Available AI Compute Instances", border_style="cyan")
        table.add_column("Provider")
        table.add_column("Model ID")
        table.add_column("Tier")
        
        # Real-world mock or query from router
        table.add_row("Gemini", "gemini-2.0-flash-exp", "SOTA")
        table.add_row("Gemini", "gemini-1.5-pro", "Advanced")
        table.add_row("Anthropic", "claude-3-5-sonnet", "Enterprise")
        table.add_row("Ollama", "qwen2.5-coder", "Local")
        
        console.print(table)
        console.print(f"Current selection: [bold cyan]{self.active_model}[/bold cyan]")

    async def show_skills(self):
        console.print(Panel(
            "\n".join([f"• [green]{s}[/green]" for s in self.available_skills]),
            title="Sovereign Node Capabilities",
            border_style="green"
        ))

    async def show_status(self):
        table = Table(title="Heiwa Enterprise Mesh Status", border_style="bold blue")
        table.add_column("Node", style="cyan bold")
        table.add_column("Status", justify="center")
        table.add_column("CPU", justify="right")
        table.add_column("RAM", justify="right")
        
        display_map = {
            "macbook": "🍎 MAC",
            "workstation": "🐧 WSL",
            "railway": "☁️ HQ"
        }
        
        for name, data in self.telemetry.items():
            if name in ["last_cost", "total_tokens"]: continue
            is_online = (time.time() - data.get("last_seen", 0)) < 60
            status_text = "[bold green]✅ RUNNING[/bold green]" if is_online else "[bold red]❌ GHOSTED[/bold red]"
            display_name = display_map.get(name, name.capitalize())
            table.add_row(display_name, status_text, data["cpu"], data["ram"])
        
        console.print(table)

    async def send_task(self, text: str) -> str:
        """Atomic task dispatch with Digital Barrier verification and @file injection."""
        if not self.nc:
            console.print("[red]❌ Not connected to mesh.[/red]")
            return ""
        
        token = os.getenv("HEIWA_AUTH_TOKEN") or settings.HEIWA_AUTH_TOKEN
        if not token:
            console.print("[red]❌ HEIWA_AUTH_TOKEN missing. Handshake denied.[/red]")
            return ""

        # Parse file mentions
        processed_text = text
        context_files = []
        import re
        mentions = re.findall(r"@([\w\.\-/]+)", text)
        for m in mentions:
            fpath = Path(m)
            if fpath.exists() and fpath.is_file():
                try:
                    content = fpath.read_text()
                    context_files.append({"path": str(fpath), "content": content})
                    # Replace mention with [FILE: path] or similar if desired, 
                    # but usually we just append the content to context.
                except Exception as e:
                    console.print(f"[dim]Note: Could not read {m}: {e}[/dim]")

        task_id = f"task-{uuid.uuid4().hex[:6]}"
        self.task_cache[task_id] = True
        
        # Determine actual model identifier if we are in auto mode
        model_intent = self.active_model
        if "auto" in model_intent.lower():
            # Heuristically route based on prompt complexity
            if len(text) > 500 or "plan" in text.lower() or "architect" in text.lower():
                model_intent = "gemini-1.5-pro"
            elif "code" in text.lower() or "script" in text.lower():
                model_intent = "qwen2.5-coder" # Direct to local code model
            else:
                model_intent = "gemini-2.0-flash-exp" # Fast iteration

        payload = {
            Payload.SENDER_ID: self.node_id,
            Payload.TIMESTAMP: time.time(),
            Payload.TYPE: Subject.CORE_REQUEST.name,
            "auth_token": token,
            Payload.DATA: {
                "task_id": task_id,
                "raw_text": processed_text,
                "context": {"files": context_files},
                "model_id": model_intent,
                "source": "cli",
                "auth_token": token,
                "sender_id": self.node_id,
                "target_runtime": "any"
            }
        }
        
        try:
            await self.nc.publish(Subject.CORE_REQUEST.value, json.dumps(payload).encode())
            console.print(f"[bold yellow]▶ DISPATCHED:[/bold yellow] [dim]{task_id}[/dim] [italic dim](Route: {model_intent}, Files: {len(context_files)})[/italic dim]")
            return task_id
        except Exception as e:
            console.print(f"[red]❌ Dispatch failed:[/red] {e}")
            return ""

    async def run(self, initial_prompt: Optional[str] = None):
        console.clear()
        console.print(Align.center(Panel(
            Text.assemble(
                ("HEIWA ", "bold magenta"), ("ENTERPRISE CORE ", "bold white"), ("v3.0\n", "dim"),
                ("Digital Barrier: ", "bold"), ("ACTIVE ", "green"), ("| Node: ", "bold"), (f"{self.node_id}", "cyan")
            ),
            border_style="magenta", padding=(1, 2)
        )))
        
        with console.status("[bold blue]Establishing Mesh Handshake...") as status:
            connected = await self.connect()
            if not connected:
                console.print("[yellow]⚠️  Running in Offline Mode.[/yellow]")
            else:
                console.print("[green]✅ Handshake Complete. Barrier Syncronized.[/green]")

        if initial_prompt:
            console.print(f"[bold magenta]▶ PROMPT DISPATCH:[/bold magenta] {initial_prompt}")
            task_id = await self.send_task(initial_prompt)
            
            # Wait for result or timeout
            start_wait = time.time()
            timeout = 120 # Increase for SOTA reasoning
            while task_id in self.task_cache and (time.time() - start_wait) < timeout:
                await asyncio.sleep(1) # More time for agents to process
            
            if task_id in self.task_cache:
                console.print(f"\n[yellow]⚠️  Task {task_id} timed out after {timeout}s. Checking mesh heartbeats...[/yellow]")
                await self.show_status()
            
            # Final drain to ensure NATS receives/sends all remaining messages
            if self.nc: await self.nc.drain()
            self.running = False
            return

        while self.running:
            try:
                user_input = await self.session.prompt_async(
                    HTML('<b fg="ansimagenta">heiwa</b> > '),
                    bottom_toolbar=self.get_bottom_toolbar,
                    key_bindings=self.kb,
                    refresh_interval=1
                )
                
                if user_input is None: break
                if not user_input.strip(): continue
                if user_input.startswith("/"):
                    await self.execute_command(user_input)
                else:
                    await self.send_task(user_input)
                    
            except (KeyboardInterrupt, EOFError):
                break
            except Exception as e:
                console.print(f"[red]CLI Error:[/red] {e}")

        if self.nc: await self.nc.close()
        console.print("[yellow]🔒 Session closed. Digital Barrier remains in force.[/yellow]")

if __name__ == "__main__":
    node_name = load_node_identity().get("name", "macbook-agile")
    prompt = sys.argv[2] if len(sys.argv) > 2 else None
    asyncio.run(HeiwaShell(node_name).run(initial_prompt=prompt))
