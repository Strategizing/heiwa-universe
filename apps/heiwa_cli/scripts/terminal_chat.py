import asyncio
import json
import os
import sys
import uuid
import datetime
import time
from pathlib import Path
from typing import Any, Dict, List, Optional

# Load Environment
try:
    from dotenv import load_dotenv
    load_dotenv(Path(__file__).resolve().parents[3] / ".env")
except ImportError:
    pass

# Ensure runtime libs can be imported
ROOT = Path(__file__).resolve().parents[3]
if str(ROOT / "packages/heiwa_sdk") not in sys.path:
    sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))
if str(ROOT / "packages/heiwa_protocol") not in sys.path:
    sys.path.insert(0, str(ROOT / "packages/heiwa_protocol"))
if str(ROOT / "apps") not in sys.path:
    sys.path.insert(0, str(ROOT / "apps"))

import nats
from nats.aio.client import Client as NATSClient
from heiwa_protocol.protocol import Subject
from heiwa_sdk.ui import UIManager
from heiwa_sdk.db import Database

from rich.console import Console
from rich.markdown import Markdown
from rich.panel import Panel
from rich.live import Live
from rich.layout import Layout
from rich.table import Table
from rich.text import Text
from rich.align import Align
from rich.status import Spinner

from prompt_toolkit import PromptSession
from prompt_toolkit.history import FileHistory
from prompt_toolkit.auto_suggest import AutoSuggestFromHistory
from prompt_toolkit.completion import Completer, Completion
from prompt_toolkit.styles import Style as PromptStyle
from prompt_toolkit.key_binding import KeyBindings

console = Console()

class HeiwaCompleter(Completer):
    """Custom completer for Heiwa CLI commands and local files."""
    def __init__(self):
        self.commands = ["/status", "/cost", "/nodes", "/clear", "/models", "/exit", "/help", "/sync"]

    def get_completions(self, document, complete_event):
        text = document.text_before_cursor
        if text.startswith("/"):
            for cmd in self.commands:
                if cmd.startswith(text):
                    yield Completion(cmd, start_position=-len(text))
        else:
            # File completion
            path = Path(".")
            word = text.split()[-1] if text.strip() else ""
            if word:
                try:
                    for file in path.glob(f"{word}*"):
                        yield Completion(str(file), start_position=-len(word))
                except: pass

class HeiwaShell:
    """
    Enterprise-Grade Heiwa CLI.
    Full AI support, multi-node telemetry, and direct mesh access.
    """
    def __init__(self, node_id: str):
        self.node_id = node_id
        self.nc: NATSClient = nats.NATS()
        self.running = True
        self.db = Database()
        self.task_cache: Dict[str, Any] = {}
        self.telemetry = {
            "macbook": {"cpu": "0%", "ram": "0%"},
            "workstation": {"cpu": "OFFLINE", "ram": "OFFLINE"},
            "railway": {"cpu": "0%", "ram": "0%"},
            "last_cost": "$0.0000"
        }
        
        # UI State
        self.live_display = None
        self.current_response = ""
        self.is_thinking = False
        
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
            event.app.exit()

    async def connect(self, max_retries: int = 5, retry_delay: int = 3):
        nats_url = os.getenv("NATS_URL", "nats://localhost:4222")
        for attempt in range(1, max_retries + 1):
            try:
                await self.nc.connect(nats_url, connect_timeout=10)
                console.print(f"[green]✅ Connected to Heiwa Mesh:[/green] {nats_url}")
                break
            except Exception as e:
                if attempt == max_retries:
                    console.print(f"[red]❌ Mesh Connection Failed after {max_retries} attempts:[/red] {e}")
                    self.nc = None
                else:
                    console.print(f"[yellow]🔄 Retrying Mesh Connection ({attempt}/{max_retries})...[/yellow]")
                    await asyncio.sleep(retry_delay)

        # Swarm Subscriptions
        if self.nc:
            await self.nc.subscribe(Subject.TASK_EXEC_RESULT.value, cb=self.handle_result)
            await self.nc.subscribe(Subject.LOG_THOUGHT.value, cb=self.handle_thought)
            await self.nc.subscribe(Subject.NODE_TELEMETRY.value, cb=self.handle_telemetry)
            await self.nc.subscribe(Subject.TASK_PROGRESS.value, cb=self.handle_progress)

    def generate_footer(self) -> Table:
        table = Table.grid(expand=True)
        table.add_column(justify="left", ratio=1)
        table.add_column(justify="right", ratio=1)
        
        m = self.telemetry["macbook"]
        w = self.telemetry["workstation"]
        r = self.telemetry["railway"]
        
        active_tasks = len(self.task_cache)
        task_color = "green" if active_tasks == 0 else "yellow"
        
        left_text = Text.assemble(
            (f" 🍎 Mac: ", "bold cyan"), (f"{m['cpu']} CPU ", "white"),
            (f" 🪟 PC: ", "bold green"), (f"{w['cpu']} CPU ", "white"),
            (f" ☁️ Cloud: ", "bold magenta"), (f"{r['cpu']} CPU ", "white"),
            (f" | ⚡ Parallel Agents: ", "bold"), (f"{active_tasks}", f"bold {task_color}")
        )
        
        right_text = Text.assemble(
            (f"💰 24h Spend: ", "bold yellow"), (f"{self.telemetry['last_cost']} ", "white"),
            (f" {datetime.datetime.now().strftime('%H:%M:%S')} ", "dim")
        )
        
        table.add_row(left_text, right_text)
        return table

    async def handle_result(self, msg):
        payload = json.loads(msg.data.decode())
        data = payload.get("data", {})
        task_id = data.get("task_id")
        
        if task_id in self.task_cache:
            self.is_thinking = False
            summary = data.get("summary", "")
            console.print("\n")
            console.print(Panel(Markdown(summary), title=f"Result: {task_id}", border_style="green", subtitle=f"Node: {data.get('runtime')} | Tool: {data.get('target_tool')}"))
            del self.task_cache[task_id]

    async def handle_thought(self, msg):
        payload = json.loads(msg.data.decode())
        data = payload.get("data", {})
        content = data.get("content", "")
        agent = payload.get("agent") or data.get("agent", "unknown")
        task_id = data.get("task_id", "swarm")
        
        # Display thoughts with a distinct style
        console.print(f"[bold black on cyan] 🧠 {agent} [/bold black on cyan] [dim]({task_id}):[/dim] [italic]{content[:120]}...[/italic]")

    async def handle_telemetry(self, msg):
        payload = json.loads(msg.data.decode()).get("data", {})
        nid = payload.get("node_id", "unknown")
        cpu = f"{payload.get('cpu_pct', 0)}%"
        ram = f"{payload.get('ram_pct', 0)}%"
        
        if "macbook" in nid: self.telemetry["macbook"] = {"cpu": cpu, "ram": ram}
        elif "wsl" in nid or "workstation" in nid: self.telemetry["workstation"] = {"cpu": cpu, "ram": ram}
        elif "railway" in nid: self.telemetry["railway"] = {"cpu": cpu, "ram": ram}

    async def handle_progress(self, msg):
        payload = json.loads(msg.data.decode()).get("data", {})
        if payload.get("task_id") in self.task_cache:
            console.print(f"[dim]⏳ {payload.get('content', '...')}[/dim]")

    async def execute_command(self, cmd: str):
        if cmd == "/clear":
            console.clear()
        elif cmd == "/status":
            await self.show_status()
        elif cmd == "/cost":
            await self.show_cost()
        elif cmd == "/nodes":
            await self.show_nodes()
        elif cmd == "/models":
            await self.show_models()
        elif cmd == "/exit":
            self.running = False
        elif cmd == "/sync":
            console.print("[yellow]Syncing swarm structure...[/yellow]")
            # Publish a sync request to NATS
        elif cmd == "/help":
            console.print(Markdown("# Heiwa CLI Commands\n- `/status`: Swarm health\n- `/cost`: Spending report\n- `/nodes`: Active hardware\n- `/models`: Available intelligence\n- `/clear`: Clear terminal\n- `/exit`: Close session"))

    async def show_cost(self):
        summary = self.db.get_model_usage_summary(minutes=1440)
        table = Table(title="Swarm Cost Analysis (24h)", header_style="bold yellow")
        table.add_column("Model")
        table.add_column("Requests", justify="right")
        table.add_column("Tokens", justify="right")
        for row in summary:
            table.add_row(row['model_id'], str(row['request_count']), f"{row['total_tokens']:,}")
        console.print(table)

    async def show_nodes(self):
        table = Table(title="Heiwa Swarm Nodes", header_style="bold cyan")
        table.add_column("Node ID")
        table.add_column("Status")
        table.add_column("Type")
        table.add_row("macbook@heiwa-agile", "[green]Online[/green]", "M4 Pro")
        table.add_row("wsl@heiwa-thinker", "[yellow]Polling...[/yellow]", "RTX 3060")
        table.add_row("railway@mesh-brain", "[green]Online[/green]", "32GB Cloud")
        console.print(table)

    async def show_status(self):
        console.print(Panel(self.generate_footer(), title="Swarm Telemetry", border_style="magenta"))
        await self.show_nodes()

    async def show_models(self):
        manifest_path = ROOT / "core" / "config" / "swarm_manifest.json"
        if not manifest_path.exists():
            console.print("[red]❌ Swarm manifest not found.[/red]")
            return
        
        with open(manifest_path, "r") as f:
            manifest = json.load(f)
        
        table = Table(title="Available Model Instances", header_style="bold green")
        table.add_column("Model ID")
        table.add_column("Primary Host")
        table.add_column("Capability")
        table.add_column("Tier")
        
        for inst in manifest.get("instances", []):
            table.add_row(
                inst["model_id"],
                inst["host_node"],
                inst["capability"],
                inst["cost_tier"]
            )
        console.print(table)

    async def send_task(self, text: str):
        if not self.nc:
            console.print("[red]❌ Not connected to mesh. Cannot dispatch tasks.[/red]")
            return

        # Parse File Mentions
        mentions = []
        words = text.split()
        for word in words:
            try:
                p = Path(word)
                if p.is_file():
                    content = p.read_text(errors="ignore")
                    mentions.append(f"\n--- FILE: {word} ---\n{content}\n")
            except: pass
        
        final_text = text + "".join(mentions)
        task_id = f"cli-{uuid.uuid4().hex[:8]}"
        self.task_cache[task_id] = True
        self.is_thinking = True
        
        payload = {
            Payload.SENDER_ID: self.node_id,
            Payload.TIMESTAMP: time.time(),
            Payload.TYPE: Subject.CORE_REQUEST.name,
            Payload.DATA: {
                "task_id": task_id,
                "raw_text": final_text,
                "source": "cli",
                "intent_class": "general",
                "target_runtime": "any"
            }
        }
        
        await self.nc.publish(Subject.CORE_REQUEST.value, json.dumps(payload).encode())
        console.print(f"[bold yellow]▶ DISPATCHED:[/bold yellow] {task_id}")

    async def run(self):
        await self.connect()
        
        console.print(Align.center(Panel(
            Text.assemble(
                ("HEIWA ", "bold magenta"), ("ENTERPRISE CLI ", "bold white"), ("v2.0\n", "dim"),
                ("Mesh Brain: ", "bold"), ("100.69.191.35 ", "cyan"), ("| Node: ", "bold"), (f"{self.node_id}", "green")
            ),
            border_style="magenta",
            padding=(1, 2)
        )))

        # Start live footer
        with Live(self.generate_footer(), refresh_per_second=1, vertical_overflow="visible") as live:
            self.live_display = live
            while self.running:
                try:
                    # Update footer
                    live.update(self.generate_footer())
                    
                    # Prompt for input
                    user_input = await self.session.prompt_async(
                        "\nheiwa > ",
                        style=PromptStyle.from_dict({'prompt': 'bold blue'}),
                        key_bindings=self.kb
                    )
                    
                    if not user_input.strip(): continue
                    if user_input.lower() in ["/exit", "/quit"]:
                        self.running = False
                        continue
                    
                    if user_input.startswith("/"):
                        await self.execute_command(user_input)
                    else:
                        await self.send_task(user_input)
                        
                except (KeyboardInterrupt, EOFError):
                    self.running = False
                except Exception as e:
                    console.print(f"[red]CLI Error:[/red] {e}")

        if self.nc:
            await self.nc.close()
        console.print("[yellow]🔒 Session closed.[/yellow]")

if __name__ == "__main__":
    node_name = sys.argv[1] if len(sys.argv) > 1 else "macbook@heiwa-agile"
    shell = HeiwaShell(node_name)
    asyncio.run(shell.run())