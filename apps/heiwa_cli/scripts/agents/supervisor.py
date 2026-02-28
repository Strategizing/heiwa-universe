#!/usr/bin/env python3
# cli/scripts/agents/supervisor.py
from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path
from typing import Dict, List, Optional

import yaml

class PolicyError(RuntimeError):
    pass

class Supervisor:
    def __init__(self, config_path: str = "config/agents.yaml"):
        self.repo = Path(os.environ.get("HEIWA_WORKSPACE_ROOT", ".")).resolve()
        self.cfg = self._load_yaml(self.repo / config_path)
        self.policy = self.cfg["policy"]
        self.budgets = self.cfg["budgets"]
        self.tests = self.cfg.get("tests", {})

    def route_task(self, task_payload: str, complexity_score: int = 1) -> str:
        if len(task_payload) > int(self.budgets["max_prompt_chars"]):
            raise PolicyError("Task payload exceeds max_prompt_chars budget")

        threshold = int(self.policy["escalation_threshold"])
        if complexity_score <= threshold:
            return self.exec_local_ollama(task_payload)
        return self.exec_codex(task_payload)

    def exec_local_ollama(self, payload: str) -> str:
        prov = self.cfg["providers"]["ollama"]
        model = prov["default_model"]
        # Placeholder: call your wrapper, not direct provider logic.
        # Example: subprocess.run(["ollama", "run", model, payload], check=True, text=True)
        return f"[SIM] ollama:{model} would handle: {payload[:120]}..."

    def exec_codex(self, payload: str) -> str:
        prov = self.cfg["providers"]["codex"]
        if not prov.get("enabled", False):
            raise PolicyError("Codex escalation is disabled")
        cmd = prov.get("command", "codex")
        mode = prov.get("mode", "agent")
        # Placeholder:
        # subprocess.run([cmd, mode, payload], check=True, text=True)
        return f"[SIM] codex:{mode} would handle: {payload[:120]}..."

    # --- Self-brick guardrail primitives ---

    def assert_path_allowed(self, rel_path: str) -> None:
        rel = Path(rel_path)
        for pat in self.policy.get("protected_paths", []):
            if rel.match(pat) or str(rel).startswith(pat.rstrip("/")):
                raise PolicyError(f"Protected path modification blocked: {rel}")

    def stage_patch(self, rel_paths: List[str]) -> Path:
        stage = Path(self.policy["stage_dir"]).resolve()
        if stage.exists():
            shutil.rmtree(stage)
        stage.mkdir(parents=True, exist_ok=True)

        for rel in rel_paths:
            self.assert_path_allowed(rel)
            src = self.repo / rel
            if not src.exists():
                raise PolicyError(f"Source file missing: {rel}")
            dst = stage / rel
            dst.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(src, dst)

        return stage

    def run_tests(self) -> None:
        if not self.policy.get("require_tests_on_write", True):
            return

        cmds: List[str] = []
        cmds += self.tests.get("lint", [])
        cmds += self.tests.get("unit", [])

        for c in cmds:
            self._run_shell(c, cwd=self.repo)

    def apply_staged(self, stage_dir: Path) -> None:
        # Copy staged files into repo (fail-closed via tests pre-apply)
        for p in stage_dir.rglob("*"):
            if p.is_file():
                rel = p.relative_to(stage_dir)
                self.assert_path_allowed(str(rel))
                dst = self.repo / rel
                dst.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(p, dst)

    # --- helpers ---

    @staticmethod
    def _load_yaml(path: Path) -> Dict:
        return yaml.safe_load(path.read_text(encoding="utf-8"))

    @staticmethod
    def _run_shell(cmd: str, cwd: Path) -> None:
        subprocess.run(cmd, cwd=str(cwd), shell=True, check=True)

if __name__ == "__main__":
    sup = Supervisor()
    print(sup.route_task("Fix crash loop in muscle_node.py", complexity_score=2))