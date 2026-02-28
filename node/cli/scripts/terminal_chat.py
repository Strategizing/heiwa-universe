import asyncio
import json
import os
import sys
import uuid
from pathlib import Path
from typing import Any, Dict

# Ensure project root is on sys.path
ROOT = Path(__file__).resolve().parents[3]
RUNTIME_ROOT = ROOT / "runtime"
if str(RUNTIME_ROOT) not in sys.path:
    sys.path.insert(0, str(RUNTIME_ROOT))

import nats
from nats.aio.client import Client as NATSClient
from fleets.hub.protocol import Subject, Payload
from rich.console import Console
from rich.markdown import Markdown
from rich.panel import Panel
from rich.live import Live
from rich.prompt import Prompt
from rich.spinner import Spinner

console = Console()

class TerminalChat:
    """
    Direct CLI access to the Heiwa Swarm.
    Bypasses external comms for high-efficiency local-to-mesh interaction.
    """
    def __init__(self, node_id: str):
        self.node_id = node_id
        self.nc: NATSClient = nats.Client()
        self.running = True
        self.task_cache: Dict[str, Any] = {}

    async def connect(self):
        nats_url = os.getenv("NATS_URL", "nats://localhost:4222")
        try:
            await self.nc.connect(nats_url)
            console.print(f"[green]✅ Connected to Heiwa Mesh:[/green] {nats_url}")
        except Exception as e:
            console.print(f"[red]❌ Connection failed:[/red] {e}")
            sys.exit(1)

        # Listen for results directed to this CLI instance
        await self.nc.subscribe(Subject.TASK_EXEC_RESULT.value, cb=self.handle_result)
        await self.nc.subscribe(Subject.LOG_THOUGHT.value, cb=self.handle_thought)

    async def handle_result(self, msg):
        payload = json.loads(msg.data.decode())
        data = payload.get("data", {})
        task_id = data.get("task_id")
        
        if task_id in self.task_cache:
            summary = data.get("summary", "")
            console.print("\n")
            console.print(Panel(Markdown(summary), title=f"Result: {task_id}", border_style="green"))
            console.print(f"[dim]Duration: {data.get('duration_ms', 0)}ms | Node: {data.get('runtime', 'unknown')}[/dim]")
            del self.task_cache[task_id]

    async def handle_thought(self, msg):
        payload = json.loads(msg.data.decode())
        data = payload.get("data", {})
        content = data.get("content", "")
        agent = data.get("agent", "unknown")
        
        # Display thoughts in a subtle way
        console.print(f"[italic cyan]🧠 {agent}:[/italic cyan] [dim]{content[:100]}...[/dim]")

    async def send_task(self, instruction: str):
        task_id = f"cli-{uuid.uuid4().hex[:8]}"
        self.task_cache[task_id] = True
        
        payload = {
            Payload.SENDER_ID: self.node_id,
            Payload.TIMESTAMP: asyncio.get_event_loop().time(),
            Payload.TYPE: Subject.CORE_REQUEST.name,
            Payload.DATA: {
                "task_id": task_id,
                "raw_text": instruction,
                "source": "cli",
                "intent_class": "general",
                "target_runtime": "any"
            }
        }
        
        await self.nc.publish(Subject.CORE_REQUEST.value, json.dumps(payload).encode())
        console.print(f"[yellow]🚀 Task dispatched: {task_id}[/yellow]")

    async def start(self):
        await self.connect()
        
        console.print(Panel.fit(
            "Heiwa Terminal Access\nType 'exit' to quit, 'help' for commands.",
            title="[bold magenta]HEIWA v2[/bold magenta]",
            border_style="magenta"
        ))

        while self.running:
            try:
                # Use prompt for input
                user_input = await asyncio.get_event_loop().run_in_executor(
                    None, lambda: Prompt.ask("\n[bold blue]heiwa[/bold blue] > ")
                )
                
                if user_input.lower() in ["exit", "quit"]:
                    self.running = False
                    continue
                
                if not user_input.strip():
                    continue
                
                await self.send_task(user_input)
                # Small sleep to allow thoughts/results to stream in
                await asyncio.sleep(0.5)
                
            except KeyboardInterrupt:
                self.running = False
            except Exception as e:
                console.print(f"[red]Error:[/red] {e}")

        await self.nc.close()
        console.print("[yellow]🔒 Session closed.[/yellow]")

if __name__ == "__main__":
    node_id = sys.argv[1] if len(sys.argv) > 1 else "cli-user"
    chat = TerminalChat(node_id)
    asyncio.run(chat.start())
