
import json
import asyncio
from typing import List, Callable, Dict, Any
from .models import generate_text, ModelRouter

class Competitor:
    """
    Directive 002: Competitive Reasoning Wrapper.
    "Compare multiple candidate solutions and reward relative winners."
    """

    def __init__(self, model: str = "LOCAL_R1", verifier: Callable = None):
        self.model = model
        self.verifier = verifier

    async def run_competition(self, prompt: str, n: int = 5) -> Dict[str, Any]:
        """
        Generates N candidates, scores them, and returns the winner.
        """
        candidates = []
        
        # 1. Generate N candidates
        # TODO: parallelize this
        for i in range(n):
            response = generate_text(prompt, model=self.model)
            candidates.append({
                "id": i,
                "content": response,
                "score": 0,
                "passed_verification": False
            })

        # 2. Score Candidates
        winner = None
        highest_score = -1

        for cand in candidates:
            score = 0
            # Basic Heuristics
            if self.verifier:
                try:
                    if self.verifier(cand["content"]):
                        score += 50
                        cand["passed_verification"] = True
                except:
                    pass
            
            # Length check (DeepSeek usually verbose = reasoning)
            if len(cand["content"]) > 100:
                score += 10
            
            cand["score"] = score
            
            if score > highest_score:
                highest_score = score
                winner = cand

        # 3. Formulate Memory Artifact (Directive 003)
        artifact = {
            "question": prompt,
            "winner": winner,
            "candidates_count": n,
            "candidates_discarded": [c for c in candidates if c["id"] != winner["id"]],
            "confidence_score": highest_score
        }

        return artifact
