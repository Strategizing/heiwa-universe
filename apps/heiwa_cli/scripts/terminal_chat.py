"""
Heiwa Interactive Shell v5 — HTTP/WebSocket transport.

Sends tasks to the Railway hub via POST /tasks, streams results via
WS /ws/tasks/{task_id}. Falls back to direct local execution when
the hub is unreachable.
"""

import asyncio
import json
import logging
import os
import sys
import uuid
import time
from pathlib import Path
from typing import Any, Dict, Optional

# --- BOOTSTRAP ---
def _find_root(start: Path) -> Path:
    explicit = os.environ.get("HEIWA_ROOT")
    if explicit:
        p = Path(explicit).resolve()
        if (p / "apps").exists() and (p / "packages").exists():
            return p
    current = start.resolve()
    for _ in range(5):
        if (current / "apps").exists() and (current / "packages").exists():
            return current
        current = current.parent
    print("[FATAL] Cannot find Heiwa monorepo root. Set HEIWA_ROOT.", file=sys.stderr)
    sys.exit(1)

ROOT = _find_root(Path(__file__).resolve())
for pkg in ["heiwa_sdk", "heiwa_protocol", "heiwa_identity", "heiwa_ui"]:
    path = str(ROOT / f"packages/{pkg}")
    if path not in sys.path:
        sys.path.insert(0, path)
if str(ROOT / "apps") not in sys.path:
    sys.path.insert(0, str(ROOT / "apps"))

from heiwa_sdk.config import hub_url_candidates, load_swarm_env, settings
load_swarm_env()

from heiwa_sdk.db import Database
from heiwa_sdk.operator_surface import WELCOME_SUGGESTIONS, maybe_fast_path_turn, operator_display_name

from rich.console import Console
from rich.columns import Columns
from rich.markdown import Markdown
from rich.panel import Panel
from rich.table import Table
from rich.text import Text
from rich.align import Align

from prompt_toolkit import PromptSession
from prompt_toolkit.history import FileHistory
from prompt_toolkit.auto_suggest import AutoSuggestFromHistory
from prompt_toolkit.completion import Completer, Completion
from prompt_toolkit.formatted_text import HTML
from prompt_toolkit.patch_stdout import patch_stdout

console = Console()
logger = logging.getLogger("heiwa.cli.terminal_chat")


def _render_output_text(output: str) -> str:
    text = str(output or "").strip()
    if not text:
        return ""
    start = text.find("{")
    end = text.rfind("}")
    if start != -1 and end > start:
        try:
            payload = json.loads(text[start : end + 1])
        except json.JSONDecodeError:
            return text
        payloads = payload.get("payloads")
        if isinstance(payloads, list):
            rendered = []
            for item in payloads:
                if isinstance(item, dict):
                    rendered_text = str(item.get("text") or "").strip()
                    if rendered_text:
                        rendered.append(rendered_text)
            if rendered:
                return "\n\n".join(rendered)
    return text


class HeiwaCompleter(Completer):
    COMMANDS = ["/status", "/cost", "/clear", "/models", "/tips", "/exit", "/help", "/model"]

    def get_completions(self, document, complete_event):
        text = document.text_before_cursor
        if text.startswith("/"):
            for cmd in self.COMMANDS:
                if cmd.startswith(text):
                    yield Completion(cmd, start_position=-len(text))


class HeiwaShell:
    """Interactive shell that communicates with the Railway hub via HTTP + WebSocket."""

    def __init__(self, node_name: str):
        self.node_name = node_name
        self.operator_name = operator_display_name(node_name)
        self.running = True
        self.db = Database()
        self.hub_candidates = hub_url_candidates()
        self.hub_url = self.hub_candidates[0]
        self.auth_token = os.getenv("HEIWA_AUTH_TOKEN") or getattr(settings, "HEIWA_AUTH_TOKEN", "") or ""
        self.active_model = os.getenv("HEIWA_MODEL", "auto")
        self.execution_mode = "standby"
        self.last_intent = "idle"
        self.last_tool = "idle"
        self.last_transport = "idle"
        self.last_status = "ready"
        self.last_route_model = "auto"
        self.last_latency_ms: int | None = None
        self.turn_count = 0

        self.history_path = Path.home() / ".heiwa" / "cli_history"
        self.history_path.parent.mkdir(parents=True, exist_ok=True)
        self.session = PromptSession(
            history=FileHistory(str(self.history_path)),
            auto_suggest=AutoSuggestFromHistory(),
            completer=HeiwaCompleter(),
            refresh_interval=0.5,
            bottom_toolbar=self._bottom_toolbar,
        )

    def _bottom_toolbar(self):
        hub_label = self.hub_url.replace("https://", "").replace("http://", "").strip("/") or "offline"
        latency = f"{self.last_latency_ms}ms" if self.last_latency_ms is not None else "--"
        return HTML(
            "<style fg='ansigray'>mode </style>"
            f"<style fg='ansicyan'>{self.execution_mode}</style>"
            "<style fg='ansigray'>  hub </style>"
            f"<style fg='ansiblue'>{hub_label}</style>"
            "<style fg='ansigray'>  route </style>"
            f"<style fg='ansigreen'>{self.last_intent}</style>"
            "<style fg='ansigray'>  tool </style>"
            f"<style fg='ansiyellow'>{self.last_tool}</style>"
            "<style fg='ansigray'>  status </style>"
            f"<style fg='ansimagenta'>{self.last_status}</style>"
            "<style fg='ansigray'>  latency </style>"
            f"<style fg='ansiwhite'>{latency}</style>"
            "<style fg='ansigray'>  turns </style>"
            f"<style fg='ansiwhite'>{self.turn_count}</style>"
        )

    def _set_route_state(
        self,
        *,
        mode: str,
        intent: str,
        tool: str,
        transport: str,
        status: str,
        route_model: str | None = None,
        latency_ms: int | None = None,
    ) -> None:
        self.execution_mode = mode
        self.last_intent = intent or "unknown"
        self.last_tool = tool or "unknown"
        self.last_transport = transport or "unknown"
        self.last_status = status or "unknown"
        if route_model:
            self.last_route_model = route_model
        self.last_latency_ms = latency_ms

    def _show_welcome(self) -> None:
        suggestions = "\n".join(f"- `{prompt}`" for prompt in WELCOME_SUGGESTIONS)
        tips_panel = Panel(
            Markdown(
                "### Try these first\n"
                f"{suggestions}\n\n"
                "`/status` hub health  •  `/tips` show suggestions  •  `/exit` leave shell"
            ),
            title="Suggested Work",
            border_style="cyan",
        )
        shell_panel = Panel(
            Markdown(
                "### HEIWA CLI\n"
                "One operator ingress for chat, research, build, review, deploy, and loop work.\n\n"
                "The shell stays minimal. Routing, agents, and tools stay visible only when they add value."
            ),
            title=f"Session · {self.operator_name}",
            border_style="blue",
        )
        console.print(Columns([shell_panel, tips_panel], expand=True))

    # --- Hub Communication ---

    async def _submit_task(self, text: str) -> Optional[dict]:
        """POST /tasks to the Railway hub."""
        task_id = f"cli-task-{uuid.uuid4().hex[:8]}"
        body = json.dumps({
            "raw_text": text,
            "sender_id": self.node_name,
            "source_surface": "cli",
            "task_id": task_id,
        }).encode()
        headers = {
            "Content-Type": "application/json",
            "Authorization": f"Bearer {self.auth_token}",
        }

        last_error = None
        for candidate in self.hub_candidates:
            try:
                import httpx
                async with httpx.AsyncClient(timeout=10.0) as client:
                    resp = await client.post(f"{candidate}/tasks", content=body, headers=headers)
                    if resp.status_code == 200:
                        self.hub_url = candidate
                        return resp.json()
                    last_error = f"HTTP {resp.status_code}"
                    continue
            except ImportError:
                import urllib.request
                import urllib.error
                req = urllib.request.Request(
                    f"{candidate}/tasks", data=body, headers=headers, method="POST",
                )
                try:
                    with urllib.request.urlopen(req, timeout=10) as resp:
                        self.hub_url = candidate
                        return json.loads(resp.read().decode())
                except (urllib.error.URLError, urllib.error.HTTPError, OSError) as e:
                    last_error = str(e)
                    continue
            except Exception as e:
                last_error = str(e)
                continue
        console.print(f"[red]Hub unreachable: {last_error or 'no working hub candidate'}[/red]")
        return None

    async def _stream_result(self, task_id: str) -> None:
        """Connect to WS /ws/tasks/{task_id}?token=... and print events."""
        ws_url = self.hub_url.replace("https://", "wss://").replace("http://", "ws://")
        ws_url = f"{ws_url}/ws/tasks/{task_id}?token={self.auth_token}"

        try:
            import websockets
        except ImportError:
            console.print("[dim]websockets not installed — result streaming unavailable.[/dim]")
            return

        try:
            async with websockets.connect(ws_url, open_timeout=10) as ws:
                deadline = time.time() + 120
                while time.time() < deadline:
                    try:
                        raw = await asyncio.wait_for(ws.recv(), timeout=35.0)
                    except asyncio.TimeoutError:
                        continue
                    event = json.loads(raw)
                    status = event.get("status", "")
                    evt_type = event.get("type", "")

                    if evt_type == "heartbeat":
                        continue
                    if status in {"ACKNOWLEDGED", "DISPATCHED_PLAN", "DISPATCHED_FALLBACK"}:
                        self.last_status = status.lower()
                        console.print(f"[dim]{event.get('message', status)}[/dim]")
                    elif status == "AWAITING_APPROVAL":
                        self.last_status = "awaiting_approval"
                        console.print(
                            f"[yellow]Awaiting approval for {task_id}.[/yellow] "
                            f"[dim]Use `heiwa approve {task_id}` or `heiwa reject {task_id}`.[/dim]"
                        )
                        return
                    elif status in {"REJECTED", "EXPIRED"}:
                        self.last_status = status.lower()
                        console.print(f"[yellow]{status}: {_render_output_text(event.get('message', '')) or 'approval halted execution'}[/yellow]")
                        return
                    elif status in {"DELIVERED", "PASS"}:
                        self.last_status = "pass"
                        summary = event.get("summary", "")
                        if summary:
                            console.print(Panel(Markdown(_render_output_text(summary)), border_style="green"))
                        return
                    elif status in {"FAIL", "BLOCKED_AUTH", "BLOCKED_NO_CONTENT"}:
                        self.last_status = status.lower()
                        console.print(f"[red]{status}: {_render_output_text(event.get('message', ''))}[/red]")
                        return
                    else:
                        content = event.get("content") or event.get("message") or ""
                        if content:
                            console.print(f"[dim]{content}[/dim]")
        except Exception as e:
            logger.debug("WS stream error: %s", e)
            await self._poll_result(task_id)

    async def _poll_result(self, task_id: str, timeout: float = 120.0) -> None:
        headers = {"Authorization": f"Bearer {self.auth_token}"} if self.auth_token else {}
        deadline = time.time() + timeout

        async def _httpx_poll() -> tuple[int | None, Dict[str, Any] | None]:
            import httpx
            async with httpx.AsyncClient(timeout=10.0) as client:
                resp = await client.get(f"{self.hub_url}/tasks/{task_id}", headers=headers)
                return resp.status_code, resp.json() if resp.status_code == 200 else None

        while time.time() < deadline:
            try:
                status_code, payload = await _httpx_poll()
            except Exception:
                status_code, payload = None, None

            if status_code == 200 and isinstance(payload, dict):
                status = str(payload.get("status") or payload.get("run_status") or "").upper()
                summary = str(payload.get("summary") or payload.get("result") or payload.get("content") or "").strip()
                if status in {"PASS", "SUCCESS", "COMPLETED"}:
                    self.last_status = "pass"
                    if summary:
                        console.print(Panel(Markdown(_render_output_text(summary)), border_style="green"))
                    return
                if status == "AWAITING_APPROVAL":
                    self.last_status = "awaiting_approval"
                    console.print(
                        f"[yellow]Awaiting approval for {task_id}.[/yellow] "
                        f"[dim]Use `heiwa approve {task_id}` or `heiwa reject {task_id}`.[/dim]"
                    )
                    return
                if status in {"REJECTED", "EXPIRED"}:
                    self.last_status = status.lower()
                    console.print(f"[yellow]{status}: {_render_output_text(summary) or 'approval halted execution'}[/yellow]")
                    return
                if status in {"FAIL", "ERROR", "BLOCKED_AUTH", "BLOCKED_NO_CONTENT"}:
                    self.last_status = status.lower()
                    console.print(f"[red]{status}: {_render_output_text(summary) or 'task failed'}[/red]")
                    return
            await asyncio.sleep(2.0)

    async def _fetch_status_json(self, path: str) -> Dict[str, Any] | None:
        for candidate in self.hub_candidates:
            try:
                import urllib.request
                req = urllib.request.Request(f"{candidate}{path}", method="GET")
                with urllib.request.urlopen(req, timeout=5) as resp:
                    self.hub_url = candidate
                    return json.loads(resp.read().decode())
            except Exception:
                continue
        return None

    # --- Commands ---

    async def send_task(self, text: str):
        self.turn_count += 1
        fast_path = maybe_fast_path_turn(text, self.node_name)
        if fast_path:
            self._set_route_state(
                mode="scale-zero",
                intent=fast_path.intent,
                tool=fast_path.tool,
                transport="local_surface",
                status="pass",
                route_model="none",
                latency_ms=0,
            )
            console.print(Panel(Markdown(fast_path.response), title="Heiwa", border_style="cyan"))
            return

        started = time.perf_counter()
        result = await self._submit_task(text)
        if result:
            task_id = result.get("task_id", "unknown")
            route = result.get("route", {})
            route_model = str(route.get("target_model", "default"))
            self._set_route_state(
                mode="hub",
                intent=str(route.get("intent_class", "unknown")),
                tool=str(route.get("target_tool", "unknown")),
                transport="http_ws",
                status="accepted",
                route_model=route_model,
                latency_ms=int((time.perf_counter() - started) * 1000),
            )
            console.print(
                f"[dim]route {route.get('intent_class', '?')} -> "
                f"{route.get('target_tool', '?')} -> {route_model} | hub[/dim]"
            )
            await self._stream_result(task_id)
        else:
            # Fallback to direct local execution
            console.print("[yellow]Falling back to direct local execution...[/yellow]")
            from heiwa_hub.cognition.enrichment import BrokerEnrichmentService
            from heiwa_protocol.routing import BrokerRouteRequest
            from heiwa_sdk.heiwaclaw import HeiwaClawGateway

            task_id = f"cli-task-{uuid.uuid4().hex[:8]}"
            req = BrokerRouteRequest(
                request_id=f"chat-{task_id}", task_id=task_id, raw_text=text,
                sender_id=self.node_name, source_surface="cli",
                auth_validated=bool(self.auth_token),
            )
            svc = BrokerEnrichmentService()
            route = svc.enrich(req)
            gateway = HeiwaClawGateway(ROOT)
            dispatch = gateway.resolve(route)
            self._set_route_state(
                mode="direct",
                intent=route.intent_class,
                tool=dispatch.adapter_tool,
                transport=dispatch.transport,
                status="running",
                route_model=dispatch.target_model or "default",
                latency_ms=int((time.perf_counter() - started) * 1000),
            )
            console.print(
                f"[dim]route {route.intent_class} -> {dispatch.provider} -> "
                f"{dispatch.target_model or 'default'} | {dispatch.transport}[/dim]"
            )
            exit_code, output = await gateway.execute(route, text)
            self.last_status = "pass" if exit_code == 0 else "fail"
            if output:
                console.print(
                    Panel(
                        Markdown(_render_output_text(str(output).strip())),
                        border_style="green" if exit_code == 0 else "red",
                    )
                )

    async def execute_command(self, cmd: str):
        parts = cmd.split(" ", 1)
        base = parts[0]

        if base == "/exit":
            self.running = False
        elif base == "/clear":
            console.clear()
        elif base == "/model":
            if len(parts) > 1:
                self.active_model = parts[1]
                console.print(f"[green]Model set to {self.active_model}[/green]")
            else:
                await self.show_models()
        elif base == "/status":
            await self.show_status()
        elif base == "/tips":
            self._show_welcome()
        elif base == "/help":
            console.print(Markdown(
                "# Heiwa CLI\n"
                "One input box is the default. Slash commands are just operator controls.\n\n"
                "- `/status`: hub + route state\n"
                "- `/tips`: show suggested prompts\n"
                "- `/model <name>`: set an operator preference\n"
                "- `/clear`: clear the screen\n"
                "- `/exit`: close the shell"
            ))

    async def show_models(self):
        t = Table(title="Heiwa Model Routing")
        t.add_column("Provider")
        t.add_column("Model")
        t.add_column("Tier")
        t.add_row("Ollama", "llama-4-scout:q4_k_m", "Local")
        t.add_row("Google", "gemini-3-pro-preview", "Cloud / CLI")
        t.add_row("Claude", "claude-opus-4-6", "Cloud / CLI")
        t.add_row("System", "auto", "Smart Router")
        console.print(t)

    async def show_status(self):
        t = Table(title="Heiwa Status")
        t.add_column("Property")
        t.add_column("Value")
        t.add_row("Hub URL", self.hub_url)
        t.add_row("Auth", "set" if self.auth_token else "[red]not set[/red]")
        t.add_row("Active Model", self.active_model)
        t.add_row("Execution Mode", self.execution_mode)
        t.add_row("Last Route", f"{self.last_intent} -> {self.last_tool}")
        t.add_row("Last Transport", self.last_transport)
        t.add_row("Turns", str(self.turn_count))
        health = await self._fetch_status_json("/health")
        public = await self._fetch_status_json("/status")

        if health:
            t.add_row("Hub Status", f"[green]{health.get('status', 'unknown')}[/green]")
            t.add_row("Backend", str(health.get("state_backend", "unknown")))
            t.add_row("Transport", str(health.get("gateway_transport", "unknown")))
        else:
            t.add_row("Hub Status", "[red]unreachable[/red]")

        if public:
            t.add_row("Live Nodes", str(public.get("live_nodes", 0)))
            t.add_row("Active Models", str(public.get("active_models", 0)))

        console.print(t)

        rate_groups = dict((public or {}).get("rate_groups") or {})
        limited_groups = [
            (name, data)
            for name, data in rate_groups.items()
            if isinstance(data, dict) and not data.get("unlimited")
        ]
        if limited_groups:
            rt = Table(title="OAuth Rate Groups")
            rt.add_column("Group")
            rt.add_column("Used", justify="right")
            rt.add_column("Avail", justify="center")
            rt.add_column("Cooldown", justify="right")
            ordered = sorted(
                limited_groups,
                key=lambda item: (
                    bool(item[1].get("available", False)),
                    -float(item[1].get("cooldown_remaining", 0) or 0),
                    -(int(item[1].get("used", 0) or 0)),
                ),
            )
            for group, data in ordered[:6]:
                cooldown = float(data.get("cooldown_remaining", 0) or 0)
                max_turns = int(data.get("max", 0) or 0)
                used = int(data.get("used", 0) or 0)
                available = "[green]yes[/green]" if data.get("available") else "[yellow]cooldown[/yellow]"
                cooldown_text = f"{int(cooldown)}s" if cooldown > 0 else "—"
                rt.add_row(group, f"{used}/{max_turns}", available, cooldown_text)
            console.print(rt)

    # --- Main Loop ---

    async def run(self, initial_prompt: str = ""):
        console.print(Panel(
            Align.center("[bold white]HEIWA CLI[/bold white]\n[dim]single ingress | route-aware | scale-zero first[/dim]"),
            border_style="blue",
        ))

        if initial_prompt.strip():
            console.print(f"[dim]Initial prompt:[/dim] {initial_prompt}")
            await self.send_task(initial_prompt)
        else:
            self._show_welcome()

        with patch_stdout():
            while self.running:
                try:
                    identity_str = self.node_name.split("@")[0] if "@" in self.node_name else self.node_name
                    user_input = await self.session.prompt_async(
                        HTML(f'<ansicyan><b>heiwa</b></ansicyan><ansigray>@</ansigray><ansiblue>{identity_str}</ansiblue> <ansigray>></ansigray> '),
                    )
                    if not user_input.strip():
                        continue
                    if user_input.startswith("/"):
                        await self.execute_command(user_input)
                    else:
                        await self.send_task(user_input)
                except (EOFError, KeyboardInterrupt):
                    self.running = False

        console.print("\n[blue]Session closed.[/blue]")


if __name__ == "__main__":
    node_name = sys.argv[1] if len(sys.argv) > 1 else os.getenv("HEIWA_NODE_ID", "macbook@heiwa-agile")
    initial_prompt = " ".join(sys.argv[2:]).strip() if len(sys.argv) > 2 else ""
    asyncio.run(HeiwaShell(node_name).run(initial_prompt=initial_prompt))
