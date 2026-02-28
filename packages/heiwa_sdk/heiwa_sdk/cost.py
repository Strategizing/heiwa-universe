import logging

logger = logging.getLogger("SDK.Cost")

class CostEstimator:
    """
    Estimates token costs for different providers/models.
    Prices based on standard February 2026 market rates (per 1M tokens).
    """
    PRICING = {
        "google/gemini-2.5-flash": {"input": 0.10, "output": 0.40},
        "google/gemini-1.5-pro": {"input": 1.25, "output": 5.00},
        "google-gemini-cli/gemini-3-flash-preview": {"input": 0.0, "output": 0.0}, # Placeholder free
        "google-antigravity/claude-opus-4-5-thinking": {"input": 15.00, "output": 75.00},
        "openai-codex/gpt-5.2-codex": {"input": 10.00, "output": 30.00},
        "groq/llama-3.3-70b-versatile": {"input": 0.59, "output": 0.79},
        "cerebras/gpt-oss-120b": {"input": 0.10, "output": 0.10}, # Placeholder low
        "openrouter/google/gemini-2.0-flash:free": {"input": 0.0, "output": 0.0},
        "ollama/": {"input": 0.0, "output": 0.0}, # Local is free
    }

    @staticmethod
    def calculate(model_id: str, input_tokens: int, output_tokens: int) -> float:
        # Find best match in pricing table
        rate = None
        for key, val in CostEstimator.PRICING.items():
            if model_id.startswith(key):
                rate = val
                break
        
        if not rate:
            # Default to a safe high estimate for unknown cloud models
            if any(p in model_id for p in ["google", "openai", "anthropic", "groq"]):
                rate = {"input": 1.0, "output": 3.0}
            else:
                return 0.0 # Assume local or unknown free
        
        cost = (input_tokens / 1_000_000 * rate["input"]) + (output_tokens / 1_000_000 * rate["output"])
        return round(cost, 6)