"""
[ANTIGRAVITY] Heiwa Tools - Portable Tool Definitions
Shared tool implementations for autonomous agents.
"""

import os
import subprocess
import json
from typing import Dict, Any, Optional, List


class ToolError(Exception):
    """Raised when a tool execution fails."""
    pass


def read_file(path: str, start_line: int = None, end_line: int = None) -> Dict[str, Any]:
    """
    Read contents of a file.
    
    Args:
        path: Absolute path to the file.
        start_line: Optional 1-indexed start line.
        end_line: Optional 1-indexed end line (inclusive).
    
    Returns:
        Dict with 'content', 'total_lines', 'path'.
    """
    if not os.path.isabs(path):
        return {"error": f"Path must be absolute: {path}"}
    
    if not os.path.exists(path):
        return {"error": f"File not found: {path}"}
    
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as f:
            lines = f.readlines()
        
        total_lines = len(lines)
        
        if start_line is not None and end_line is not None:
            # Convert to 0-indexed
            start_idx = max(0, start_line - 1)
            end_idx = min(total_lines, end_line)
            lines = lines[start_idx:end_idx]
        
        return {
            "content": "".join(lines),
            "total_lines": total_lines,
            "path": path
        }
    except Exception as e:
        return {"error": str(e)}


def write_file(path: str, content: str, create_dirs: bool = True) -> Dict[str, Any]:
    """
    Write content to a file.
    
    Args:
        path: Absolute path to the file.
        content: Content to write.
        create_dirs: Create parent directories if they don't exist.
    
    Returns:
        Dict with 'success', 'path', 'bytes_written'.
    """
    if not os.path.isabs(path):
        return {"error": f"Path must be absolute: {path}"}
    
    try:
        if create_dirs:
            os.makedirs(os.path.dirname(path), exist_ok=True)
        
        with open(path, "w", encoding="utf-8") as f:
            bytes_written = f.write(content)
        
        return {
            "success": True,
            "path": path,
            "bytes_written": bytes_written
        }
    except Exception as e:
        return {"error": str(e)}


def grep(pattern: str, path: str, case_insensitive: bool = False) -> Dict[str, Any]:
    """
    Search for a pattern in files.
    
    Args:
        pattern: Search pattern (literal string).
        path: Directory or file to search.
        case_insensitive: Case insensitive search.
    
    Returns:
        Dict with 'matches' (list of {file, line_number, content}).
    """
    if not os.path.exists(path):
        return {"error": f"Path not found: {path}"}
    
    try:
        cmd = ["grep", "-rn"]
        if case_insensitive:
            cmd.append("-i")
        cmd.extend([pattern, path])
        
        result = subprocess.run(
            cmd,
            capture_output=True,
            text=True,
            timeout=30
        )
        
        matches = []
        for line in result.stdout.strip().split("\n"):
            if not line:
                continue
            # Format: file:line_number:content
            parts = line.split(":", 2)
            if len(parts) >= 3:
                matches.append({
                    "file": parts[0],
                    "line_number": int(parts[1]),
                    "content": parts[2]
                })
        
        return {"matches": matches, "count": len(matches)}
    except subprocess.TimeoutExpired:
        return {"error": "Search timed out"}
    except Exception as e:
        return {"error": str(e)}


def run_command(command: str, cwd: str = None, timeout: int = 60) -> Dict[str, Any]:
    """
    Run a shell command.
    
    Args:
        command: Shell command to execute.
        cwd: Working directory.
        timeout: Timeout in seconds.
    
    Returns:
        Dict with 'stdout', 'stderr', 'returncode'.
    """
    try:
        result = subprocess.run(
            command,
            shell=True,
            capture_output=True,
            text=True,
            cwd=cwd,
            timeout=timeout
        )
        
        return {
            "stdout": result.stdout,
            "stderr": result.stderr,
            "returncode": result.returncode,
            "success": result.returncode == 0
        }
    except subprocess.TimeoutExpired:
        return {"error": f"Command timed out after {timeout}s", "returncode": -1}
    except Exception as e:
        return {"error": str(e), "returncode": -1}


def list_directory(path: str, recursive: bool = False, max_depth: int = 2) -> Dict[str, Any]:
    """
    List contents of a directory.
    
    Args:
        path: Directory path.
        recursive: Include subdirectories.
        max_depth: Maximum depth for recursive listing.
    
    Returns:
        Dict with 'entries' (list of {name, type, size}).
    """
    if not os.path.exists(path):
        return {"error": f"Path not found: {path}"}
    
    if not os.path.isdir(path):
        return {"error": f"Not a directory: {path}"}
    
    try:
        entries = []
        
        def scan(dir_path, depth=0):
            if depth > max_depth:
                return
            
            for entry in os.scandir(dir_path):
                info = {
                    "name": entry.name,
                    "path": entry.path,
                    "type": "directory" if entry.is_dir() else "file"
                }
                
                if entry.is_file():
                    try:
                        info["size"] = entry.stat().st_size
                    except:
                        info["size"] = 0
                
                entries.append(info)
                
                if recursive and entry.is_dir():
                    scan(entry.path, depth + 1)
        
        scan(path)
        return {"entries": entries, "count": len(entries)}
    except Exception as e:
        return {"error": str(e)}


# Tool registry for agent invocation
TOOLS = {
    "read_file": read_file,
    "write_file": write_file,
    "grep": grep,
    "run_command": run_command,
    "list_directory": list_directory
}

def invoke_tool(name: str, **kwargs) -> Dict[str, Any]:
    """Invoke a tool by name with keyword arguments."""
    if name not in TOOLS:
        return {"error": f"Unknown tool: {name}"}
    
    return TOOLS[name](**kwargs)
