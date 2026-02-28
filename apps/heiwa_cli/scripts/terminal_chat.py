import asyncio
import json
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

class HeiwaCompleter(Completer):
    def __init__(self):
        self.commands = ["/status", "/cost", "/nodes", "/clear", "/models", "/exit", "/help", "/sync", "/private", "/audit"]

    def get_completions(self, document, complete_event):
        text = document.text_before_cursor
        if text.startswith("/"):
            for cmd in self.commands:
                if cmd.startswith(text):
                    yield Completion(cmd, start_position=-len(text))
        else:
            path = Path(".")
            word = text.split()[-1] if text.strip() else ""
            if word:
                try:
                    for file in path.glob(f"{word}*"):
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
        for attempt in range(1, max_retries + 1):
            try:
                await self.nc.connect(nats_url, connect_timeout=5)
                # Subscriptions
                await self.nc.subscribe(Subject.TASK_EXEC_RESULT.value, cb=self.handle_result)
                await self.nc.subscribe(Subject.LOG_THOUGHT.value, cb=self.handle_thought)
                await self.nc.subscribe(Subject.NODE_TELEMETRY.value, cb=self.handle_telemetry)
                return True
            except Exception as e:
                if attempt == max_retries:
                    console.print(f"[red]❌ Mesh Connection Failed:[/red] {e}")
                    self.nc = None
                else:
                    await asyncio.sleep(retry_delay)
        return False

    def get_bottom_toolbar(self):
        m = self.telemetry["macbook"]
        w = self.telemetry["workstation"]
        r = self.telemetry["railway"]
        
        active_tasks = len(self.task_cache)
        task_info = f" | ⚡ Parallel: {active_tasks}" if active_tasks > 0 else ""
        privacy_info = " | 🔒 PRIVATE" if self.privacy_mode else ""
        
        return HTML(
            f'<style bg="ansiblue" fg="white">'
            f' 🍎 Mac: {m["cpu"]} '
            f' 🪟 PC: {w["cpu"]} '
            f' ☁️ Cloud: {r["cpu"]} '
            f'{task_info}{privacy_info} | 💰 {self.telemetry["last_cost"]} '
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
        elif cmd == "/help":
            console.print(Markdown("# Heiwa SOTA Commands\n- `/status`: Full mesh health\n- `/private`: Toggle encryption\n- `/audit`: Corporate compliance check\n- `/models`: Available instances\n- `/exit`: Close session"))

    async def show_status(self):
        table = Table(title="Heiwa Enterprise Mesh Status", border_style="magenta")
        table.add_column("Node")
        table.add_column("Status")
        table.add_column("CPU")
        table.add_column("RAM")
        
        for name, data in self.telemetry.items():
            if name == "last_cost": continue
            is_online = (time.time() - data.get("last_seen", 0)) < 60
            status_text = "[green]ONLINE[/green]" if is_online else "[red]OFFLINE[/red]"
            table.add_row(name.capitalize(), status_text, data["cpu"], data["ram"])
        
        console.print(table)

    async def send_task(self, text: str):
        if not self.nc:
            console.print("[red]❌ Not connected to mesh.[/red]")
            return

        # Handle file attachments
        attachments = []
        for word in text.split():
            try:
                p = Path(word)
                if p.is_file():
                    attachments.append(f"\n--- ATTACHMENT: {word} ---\n{p.read_text(errors='ignore')}\n")
            except: pass
            
        task_id = f"task-{uuid.uuid4().hex[:6]}"
        self.task_cache[task_id] = True
        
        payload = {
            Payload.SENDER_ID: self.node_id,
            Payload.TIMESTAMP: time.time(),
            Payload.TYPE: Subject.CORE_REQUEST.name,
            "auth_token": settings.HEIWA_AUTH_TOKEN,
            Payload.DATA: {
                "task_id": task_id,
                "raw_text": text + "".join(attachments),
                "source": "cli",
                "private": self.privacy_mode,
                "auth_token": settings.HEIWA_AUTH_TOKEN # Redundant but safe
            }
        }
        await self.nc.publish(Subject.CORE_REQUEST.value, json.dumps(payload).encode())
        console.print(f"[bold yellow]▶ DISPATCHED:[/bold yellow] [dim]{task_id}[/dim]")

    async def run(self):
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

        while self.running:
            try:
                user_input = await self.session.prompt_async(
                    HTML('<b fg="ansimagenta">heiwa</b> > '),
                    bottom_toolbar=self.get_bottom_toolbar,
                    key_bindings=self.kb,
                    refresh_interval=1
                )
                
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
    asyncio.run(HeiwaShell(node_name).run())
