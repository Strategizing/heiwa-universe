import subprocess
import json
import os
from typing import Dict, Any, Optional

class ClawAdapter:
    """
    Wrapper for OpenClaw CLI to enable 'Intelligence' tasks.
    """
    
    def __init__(self, binary_path: str = "openclaw"):
        self.binary_path = binary_path

    def run(self, prompt: str, agent_id: Optional[str] = None, use_local: bool = False, timeout: int = 600) -> Dict[str, Any]:
        """
        Executes an OpenClaw agent turn.
        """
        cmd = [self.binary_path, "agent", "--message", prompt, "--json"]
        
        if agent_id:
            cmd.extend(["--agent", agent_id])
        
        if use_local:
            cmd.append("--local")
            
        if timeout:
            cmd.extend(["--timeout", str(timeout)])

        print(f"[CLAW] Executing: {' '.join(cmd)}")
        
        try:
            # We use check=True to raise an exception on non-zero exit
            # But since we want to capture JSON even on some failures, we'll handle it manually
            process = subprocess.run(
                cmd,
                capture_output=True,
                text=True,
                timeout=timeout + 10 # Buffer for process cleanup
            )
            
            stdout = process.stdout
            stderr = process.stderr
            
            # OpenClaw with --json should output a JSON object at the end
            # However, there might be other logs. We try to find the last JSON block.
            result = {}
            try:
                # Basic attempt to parse the entire stdout as JSON
                result = json.loads(stdout)
            except json.JSONDecodeError:
                # Fallback: look for the last line that looks like JSON or try to find { ... }
                lines = stdout.strip().split("\n")
                if lines:
                    for line in reversed(lines):
                        try:
                            result = json.loads(line)
                            break
                        except json.JSONDecodeError:
                            continue
            
            if not result and process.returncode != 0:
                result = {
                    "status": "error",
                    "error": stderr or "Unknown OpenClaw error",
                    "returncode": process.returncode
                }
            elif not result:
                result = {
                    "status": "partial",
                    "stdout": stdout,
                    "stderr": stderr,
                    "returncode": process.returncode
                }
                
            return result

        except subprocess.TimeoutExpired:
            return {
                "status": "error",
                "error": f"OpenClaw execution timed out after {timeout} seconds",
                "returncode": -1
            }
        except Exception as e:
            return {
                "status": "error",
                "error": str(e),
                "returncode": -1
            }

if __name__ == "__main__":
    # Quick sanity check
    adapter = ClawAdapter()
    res = adapter.run("echo 'Hello from Heiwa Adapter'", use_local=True)
    print(json.dumps(res, indent=2))
