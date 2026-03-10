from dataclasses import dataclass

@dataclass
class ComputeRoute:
    compute_class: int
    assigned_worker: str

class ComputeRouter:
    """
    ComputeRouter — (intent, risk) → compute_class mapping.
    Ensures sovereign data stays local and complex tasks get premium remote compute.
    """
    def route(self, intent_class: str, risk_level: str) -> ComputeRoute:
        """
        Maps (intent_class, risk_level) to a compute class and assigned worker hint.
        
        Compute Classes:
        - 1: CPU-first (local, <=7B models)
        - 2: GPU-justified (local, <=32B models)
        - 3: Premium remote (Gemini, Claude, Codex)
        - 4: Cloud persistence (Infrastructure ops)
        """
        
        # 1. Sovereign Protection: These intents involve sensitive system state
        # and MUST route to local compute (Class 1 or 2).
        sovereign_intents = {
            "deploy", 
            "operate", 
            "mesh_ops", 
            "self_buff", 
            "files", 
            "automate", 
            "automation",
            "audit"
        }
        
        if intent_class in sovereign_intents:
            # Force local GPU-justified for sovereign tasks to ensure privacy and reliability
            return RoutingResult(compute_class=2, assigned_worker="node_a_orchestrator")
        
        # 2. Reflexive / Low Latency: Status checks and chat stay local and lightweight.
        if intent_class in ["status_check", "chat"]:
            return RoutingResult(compute_class=1, assigned_worker="node_a_orchestrator")
            
        # 3. Development: High-risk code or sensitive builds stay local.
        # General research/builds can use premium remote.
        if intent_class == "build":
            if risk_level in ["high", "critical"]:
                return RoutingResult(compute_class=2, assigned_worker="node_a_codegen")
            return RoutingResult(compute_class=3, assigned_worker="class_3_build")
            
        # 4. Deep Research: Requires massive context (Gemini/Claude).
        if intent_class == "research":
            return RoutingResult(compute_class=3, assigned_worker="class_3_research")
            
        # 5. Architecture & Strategy: Requires advanced reasoning (Claude/DeepSeek).
        if intent_class == "strategy":
            return RoutingResult(compute_class=3, assigned_worker="class_3_strategy")

        # 6. Generative Media: Uses local GPU builder.
        if intent_class == "media":
            return RoutingResult(compute_class=2, assigned_worker="node_b_media")

        # 7. Default: Safe fallback to local orchestration.
        return RoutingResult(compute_class=2, assigned_worker="node_a_orchestrator")
