import logging
from typing import Dict, List, Optional, Any

logger = logging.getLogger("SDK.Routing")

class ModelRouter:
    """
    SOTA Model Routing Decision Engine.
    Routes tasks based on cost, context window, and reasoning depth.
    """
    
    # Model Metadata (Price in $/1M tokens, Context in K)
    MATRIX = {
        "google/gemini-2.0-flash": {"cost": 0.1, "context": 1000, "depth": 3},
        "google/gemini-1.5-pro": {"cost": 1.25, "context": 2000, "depth": 8},
        "groq/llama-3.3-70b-versatile": {"cost": 0.6, "context": 128, "depth": 5},
        "ollama/deepseek-r1:14b": {"cost": 0.0, "context": 32, "depth": 7},
        "ollama/qwen2.5-coder:7b": {"cost": 0.0, "context": 32, "depth": 4},
        "openai-codex/gpt-5.2-codex": {"cost": 10.0, "context": 128, "depth": 10},
        "anthropic/claude-3-5-sonnet": {"cost": 3.0, "context": 200, "depth": 9},
    }

    def __init__(self, use_local_only: bool = False):
        self.use_local_only = use_local_only

    def route(self, instruction: str, intent: str = "general") -> str:
        """Determines the optimal model for a given instruction."""
        
        # 1. Logic Depth Requirement
        depth_required = 1
        if any(w in instruction.lower() for w in ["fix", "bug", "complex", "refactor"]):
            depth_required = 7
        if any(w in instruction.lower() for w in ["strategy", "architect", "plan"]):
            depth_required = 9
            
        # 2. Context Window Requirement
        context_required = 4
        if len(instruction) > 10000:
            context_required = 128
            
        # 3. Filter candidates
        candidates = []
        for model, meta in self.MATRIX.items():
            if self.use_local_only and "ollama" not in model:
                continue
            if meta["depth"] >= depth_required and meta["context"] >= context_required:
                candidates.append((model, meta["cost"]))
                
        # 4. Sort by cost (ascending)
        candidates.sort(key=lambda x: x[1])
        
        if candidates:
            selected = candidates[0][0]
            logger.info(f"🎯 Routed to {selected} (Reasoning Depth: {depth_required})")
            return selected
            
        return "google/gemini-2.0-flash" # Global Fallback
