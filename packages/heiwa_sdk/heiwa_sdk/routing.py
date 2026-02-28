import logging
import json
import os
from pathlib import Path
from typing import Dict, List, Optional, Any

logger = logging.getLogger("SDK.Routing")

class ModelRouter:
    """
    SOTA Model Routing Decision Engine.
    Routes tasks based on cost, context window, and reasoning depth.
    Uses swarm.json and profiles.json for dynamic discovery.
    """
    
    def __init__(self, use_local_only: bool = False):
        self.use_local_only = use_local_only
        self.root = self._find_root()
        self.swarm = self._load_json(self.root / "config/swarm/swarm.json")
        self.profiles = self._load_json(self.root / "config/identities/profiles.json")

    def _find_root(self) -> Path:
        current = Path(__file__).resolve()
        for _ in range(5):
            if (current.parent / "apps").exists() and (current.parent / "packages").exists():
                return current.parent
            current = current.parent
        return Path("/Users/dmcgregsauce/heiwa")

    def _load_json(self, path: Path) -> dict:
        if path.exists():
            try:
                return json.loads(path.read_text(encoding="utf-8"))
            except: pass
        return {}

    def get_instances_for_identity(self, identity_id: str) -> List[Dict[str, Any]]:
        """Find all model instances capable of serving a specific identity."""
        identity = next((i for i in self.profiles.get("identities", []) if i["id"] == identity_id), None)
        if not identity:
            return []
        
        allowed_models = identity.get("models", {}).get("openclaw", [])
        instances = [inst for inst in self.swarm.get("instances", []) if inst["model_id"] in allowed_models]
        
        if self.use_local_only:
            instances = [i for i in instances if i["cost_tier"] == "free_local"]
            
        return instances

    def route(self, instruction: str, identity_id: str = "operator-general") -> Optional[Dict[str, Any]]:
        """Determines the optimal instance for a given instruction and identity."""
        instances = self.get_instances_for_identity(identity_id)
        if not instances:
            logger.warning(f"No instances found for identity {identity_id}")
            return None

        # 1. Depth Logic
        depth_required = 1
        if any(w in instruction.lower() for w in ["fix", "bug", "complex", "refactor"]):
            depth_required = 7
        if any(w in instruction.lower() for w in ["strategy", "architect", "plan"]):
            depth_required = 9

        # Filter by capability if possible (simplified mapping)
        # In a real mesh, we'd check instance capabilities against depth
        
        # Sort by cost (ascending)
        instances.sort(key=lambda x: 0 if x["cost_tier"] == "free_local" else (1 if x["cost_tier"] == "free_api" else 2))
        
        # For now, pick the first available instance that fits the tier
        selected = instances[0]
        logger.info(f"🎯 Routed {identity_id} to {selected['model_id']} on {selected['host_node']}")
        return selected
