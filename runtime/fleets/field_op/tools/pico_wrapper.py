import subprocess
import json
import logging
import shutil
from typing import Optional, Dict, Any, List

# Configure logging
logger = logging.getLogger(__name__)

class PicoWrapper:
    """
    Wrapper for the PicoClaw compiled binary.
    Allows Python agents to invoke PicoClaw as a subprocess for high-speed, low-memory tasks.
    """
    def __init__(self, binary_path: str = "picoclaw"):
        """
        Initialize the PicoClaw wrapper.
        :param binary_path: Path to the picoclaw executable. Defaults to 'picoclaw' in PATH.
        """
        self.binary_path = binary_path
        if not shutil.which(self.binary_path) and "/" not in self.binary_path:
             # If not in path and no path specified, warn but don't crash (might be in docker)
             logger.warning(f"⚠️ PicoClaw binary '{binary_path}' not found in PATH.")

    def run(self, command: str, args: List[str] = []) -> Dict[str, Any]:
        """
        Runs a PicoClaw command.
        
        :param command: The subcommand to run (e.g., 'scrape', 'search').
        :param args: List of additional arguments.
        :return: JSON response from PicoClaw or dict with stdout/error.
        """
        cmd = [self.binary_path, command] + args
        logger.debug(f" invoking: {' '.join(cmd)}")
        
        try:
            result = subprocess.run(
                cmd, 
                capture_output=True, 
                text=True, 
                timeout=30 # Safety timeout
            )
            
            if result.returncode != 0:
                logger.error(f"❌ PicoClaw Error: {result.stderr}")
                return {"error": result.stderr, "status": "failed"}
            
            # Attempt to parse JSON output
            try:
                return json.loads(result.stdout)
            except json.JSONDecodeError:
                # If not JSON, return raw text
                return {"output": result.stdout.strip(), "status": "success"}
                
        except  subprocess.TimeoutExpired:
             logger.error(f"❌ PicoClaw Timed Out")
             return {"error": "Timeout", "status": "timeout"}
        except FileNotFoundError:
            logger.error(f"❌ PicoClaw binary not found at {self.binary_path}")
            return {"error": "Binary not found", "status": "error"}
        except Exception as e:
            logger.error(f"❌ PicoClaw Exception: {e}")
            return {"error": str(e), "status": "error"}

    async def run_async(self, command: str, args: List[str] = []) -> Dict[str, Any]:
         """Async wrapper for run() to avoid blocking the event loop."""
         import asyncio
         loop = asyncio.get_event_loop()
         return await loop.run_in_executor(None, self.run, command, args)
