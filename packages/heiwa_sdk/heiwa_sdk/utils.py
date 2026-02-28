import subprocess
import time
import logging
from pathlib import Path
from dataclasses import dataclass
from typing import List, Optional, Any
from .security import redact_text

logger = logging.getLogger("SDK.Utils")

@dataclass
class CommandResult:
    args: List[str]
    cwd: str
    returncode: int
    stdout: str
    stderr: str
    duration_ms: int

    def to_dict(self) -> dict:
        return {
            "args": self.args,
            "cwd": self.cwd,
            "returncode": self.returncode,
            "duration_ms": self.duration_ms,
            "stdout": redact_text(self.stdout),
            "stderr": redact_text(self.stderr),
        }

def run_cmd(args: List[str], cwd: Optional[Path] = None, timeout: int = 120) -> CommandResult:
    """Executes a command and returns a structured result."""
    start = time.time()
    cmd_cwd = str(cwd) if cwd else os.getcwd()
    try:
        proc = subprocess.run(
            args,
            cwd=cmd_cwd,
            capture_output=True,
            text=True,
            timeout=timeout,
            check=False,
        )
        return CommandResult(
            args=args,
            cwd=cmd_cwd,
            returncode=int(proc.returncode),
            stdout=proc.stdout or "",
            stderr=proc.stderr or "",
            duration_ms=int((time.time() - start) * 1000),
        )
    except subprocess.TimeoutExpired:
        return CommandResult(
            args=args,
            cwd=cmd_cwd,
            returncode=-1,
            stdout="",
            stderr=f"Command timed out after {timeout}s",
            duration_ms=int((time.time() - start) * 1000),
        )
    except Exception as e:
        return CommandResult(
            args=args,
            cwd=cmd_cwd,
            returncode=-1,
            stdout="",
            stderr=str(e),
            duration_ms=int((time.time() - start) * 1000),
        )

import os # Ensure os is available for getcwd
