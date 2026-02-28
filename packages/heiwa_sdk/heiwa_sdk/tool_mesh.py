import asyncio
import json
import logging
import os
from pathlib import Path
from typing import Any, Dict, Optional, Tuple

logger = logging.getLogger("SDK.ToolMesh")

class ToolMesh:
    """
    Unified interface for all Heiwa AI tools (OpenClaw, PicoClaw, Codex, etc.)
    """
    def __init__(self, root_dir: Path):
        self.root = root_dir
        self.wrappers_dir = self.root / "apps/heiwa_cli/scripts/agents/wrappers"
        
    async def execute(self, tool: str, instruction: str, model: Optional[str] = None) -> Tuple[int, str]:
        """Execute a tool with given instruction and optional model override."""
        wrapper_map = {
            "heiwa_code": self.wrappers_dir / "codex_exec.sh",
            "heiwa_claw": self.wrappers_dir / "openclaw_exec.sh",
            "heiwa_reflex": self.wrappers_dir / "ollama_exec.py",
            "heiwa_ops": self.root / "apps/heiwa_cli/scripts/ops/heiwa_360_check.py",
            "heiwa_buff": self.root / "apps/heiwa_cli/scripts/ops/sota_verify.py",
            "codex": self.wrappers_dir / "codex_exec.sh", # Legacy alias
            "openclaw": self.wrappers_dir / "openclaw_exec.sh", # Legacy alias
            "ollama": self.wrappers_dir / "ollama_exec.py" # Legacy alias
        }
        
        wrapper = wrapper_map.get(tool.lower())
        if not wrapper or not wrapper.exists():
            return 2, f"❌ [SOVEREIGN MESH] Tool '{tool}' not localized in universe."
            
        # Set environment overrides for the subprocess
        env = os.environ.copy()
        if model:
            env["HEIWA_ACTIVE_MODEL"] = model
            env["OPENCLAW_MODEL"] = model # Legacy support
            
        logger.info(f"🌐 [HEIWA TOOLMESH] Invoking Localization: {tool} (Model: {model or 'Auto'})")
        
        # Use different execution methods based on extension
        if wrapper.suffix == ".sh":
            cmd = ["bash", str(wrapper), instruction]
        elif wrapper.suffix == ".py":
            py_bin = sys.executable # Assume current python
            cmd = [py_bin, str(wrapper), instruction]
        else:
            cmd = [str(wrapper), instruction]
            
        try:
            proc = await asyncio.create_subprocess_exec(
                *cmd,
                cwd=str(self.root),
                env=env,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.PIPE,
            )
            stdout, stderr = await proc.communicate()
            exit_code = proc.returncode
            output = (stdout + stderr).decode(errors="ignore").strip()
            return exit_code, output
        except Exception as e:
            return 1, f"Tool mesh execution error: {e}"

import sys # Needed for sys.executable
