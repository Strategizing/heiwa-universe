import os
import sys
from pathlib import Path
from rich.console import Console
from rich.table import Table

console = Console()

def run_audit():
    console.print("[bold blue]🏢 HEIWA CORPORATE COMPLIANCE AUDIT v1.0[/bold blue]\n")
    
    root = Path(__file__).resolve().parents[4]
    
    checks = {
        "Digital Barrier (Vault)": (Path.home() / ".heiwa/vault.env").exists(),
        "Monorepo Standard (Apps)": (root / "apps").exists(),
        "Monorepo Standard (Packages)": (root / "packages").exists(),
        "Namespaced SDK": (root / "packages/heiwa_sdk/heiwa_sdk").exists(),
        "Enterprise CLI": (root / "apps/heiwa_cli/heiwa").exists(),
        "Public Surface (Web)": (root / "apps/heiwa_web/clients/web/index.html").exists(),
        "Identity Manifest": (root / "config/identities/profiles.json").exists(),
        "Soul Core": (root / "config/identities/soul/core.md").exists(),
    }
    
    table = Table(title="Corporate Compliance Checklist", border_style="cyan")
    table.add_column("Protocol")
    table.add_column("Status")
    
    all_passed = True
    for protocol, status in checks.items():
        status_text = "[green]SECURED[/green]" if status else "[red]COMPROMISED[/red]"
        table.add_row(protocol, status_text)
        if not status: all_passed = False
        
    console.print(table)
    
    if all_passed:
        console.print("\n[bold green]✅ HEIWA IS FULLY COMPLIANT. AUTONOMY ENGAGED.[/bold green]")
    else:
        console.print("\n[bold red]⚠️  COMPLIANCE BREACH DETECTED. REFINE IMMEDIATELY.[/bold red]")

if __name__ == "__main__":
    run_audit()
