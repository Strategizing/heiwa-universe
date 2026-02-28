import sys
import os
from pathlib import Path
from datetime import datetime, timedelta

# Ensure runtime libs can be imported
ROOT = Path(__file__).resolve().parents[2]
RUNTIME_ROOT = ROOT / "runtime"
if str(RUNTIME_ROOT) not in sys.path:
    sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))
    sys.path.insert(0, str(ROOT / "packages"))
    sys.path.insert(0, str(ROOT / "apps"))

from heiwa_sdk.db import Database
from rich.console import Console
from rich.table import Table
from rich.panel import Panel

console = Console()

def generate_report():
    db = Database()
    
    # 1. Swarm Summary
    summary = db.get_model_usage_summary(minutes=1440) # Last 24h
    
    if not summary:
        console.print("[yellow]No usage data found for the last 24h.[/yellow]")
        return

    table = Table(title="Heiwa Swarm: 24h Token Usage & Cost", header_style="bold magenta")
    table.add_column("Model ID", style="cyan", no_wrap=True)
    table.add_column("Requests", justify="right")
    table.add_column("Total Tokens", justify="right")
    table.add_column("Est. Cost ($)", justify="right", style="green")

    total_requests = 0
    total_tokens = 0
    total_cost = 0.0

    for row in summary:
        mid = row['model_id']
        reqs = row['request_count']
        tokens = row['total_tokens']
        
        # Pull detailed cost from runs table
        conn = db.get_connection()
        cursor = conn.cursor()
        cursor.execute("SELECT SUM(cost) FROM runs WHERE model_id = ? AND ended_at > ?", 
                      (mid, (datetime.now() - timedelta(hours=24)).isoformat()))
        cost = cursor.fetchone()[0] or 0.0
        conn.close()

        table.add_row(mid, str(reqs), f"{tokens:,}", f"${cost:.4f}")
        
        total_requests += reqs
        total_tokens += tokens
        total_cost += cost

    table.add_section()
    table.add_row("TOTAL", str(total_requests), f"{total_tokens:,}", f"${total_cost:.4f}", style="bold")

    console.print(table)
    
    # 2. Optimization Tips
    if total_cost > 1.0:
        console.print(Panel(
            "High cost detected. Suggestion: Shift 'research-scout' tasks to [bold]wsl@heiwa-thinker[/bold] "
            "to utilize local Deepseek-R1 instead of Gemini Pro.",
            title="Cost Optimization Alert",
            border_style="yellow"
        ))

if __name__ == "__main__":
    generate_report()