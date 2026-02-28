import asyncio
import json
import logging
import os
import time
from dataclasses import dataclass
from typing import Any, List, Optional, AsyncGenerator

import httpx
from tenacity import retry, stop_after_attempt, wait_exponential

from heiwa_sdk.config import settings
from heiwa_identity.node import load_node_identity

logger = logging.getLogger("SDK.Cognition")

@dataclass
class TokenUsage:
    prompt_tokens: int = 0
    completion_tokens: int = 0
    total_tokens: int = 0

class CognitionEngine:
    """
    SOTA Async Cognition Engine.
    Supports streaming, tiered routing, and multi-provider failover.
    """
    
    def __init__(self):
        self.identity = load_node_identity()
        self.client = httpx.AsyncClient(timeout=60.0)

    async def generate_stream(self, 
                               prompt: str, 
                               model: str, 
                               system: Optional[str] = None) -> AsyncGenerator[str, None]:
        """Stream tokens from the selected provider."""
        if "google/" in model:
            async for chunk in self._stream_gemini(prompt, model.split("/")[-1], system):
                yield chunk
        elif "ollama/" in model:
            async for chunk in self._stream_ollama(prompt, model.split("/")[-1], system):
                yield chunk
        elif "groq/" in model:
            async for chunk in self._stream_groq(prompt, model.split("/")[-1], system):
                yield chunk
        else:
            yield f"[ERROR] Unsupported streaming model: {model}"

    async def _stream_gemini(self, prompt: str, model_name: str, system: Optional[str]) -> AsyncGenerator[str, None]:
        url = f"https://generativelanguage.googleapis.com/v1beta/models/{model_name}:streamGenerateContent?key={settings.GOOGLE_API_KEY}"
        payload = {
            "contents": [{"parts": [{"text": prompt}]}],
            "generationConfig": {"temperature": 0.2}
        }
        if system:
            payload["systemInstruction"] = {"parts": [{"text": system}]}

        try:
            async with self.client.stream("POST", url, json=payload) as response:
                if response.status_code != 200:
                    yield f"Error: HTTP {response.status_code}"
                    return
                async for line in response.aiter_lines():
                    if line.startswith('{"candidates"'):
                        data = json.loads(line)
                        yield data["candidates"][0]["content"]["parts"][0]["text"]
        except Exception as e:
            yield f"Stream failure: {e}"

    async def _stream_ollama(self, prompt: str, model_name: str, system: Optional[str]) -> AsyncGenerator[str, None]:
        url = f"{os.getenv('HEIWA_OLLAMA_URL', 'http://127.0.0.1:11434')}/api/generate"
        payload = {
            "model": model_name,
            "prompt": prompt,
            "system": system or "",
            "stream": True
        }
        try:
            async with self.client.stream("POST", url, json=payload) as response:
                async for line in response.aiter_lines():
                    if line:
                        data = json.loads(line)
                        yield data.get("response", "")
                        if data.get("done"): break
        except Exception as e:
            yield f"Ollama Stream failure: {e}"

    async def _stream_groq(self, prompt: str, model_name: str, system: Optional[str]) -> AsyncGenerator[str, None]:
        url = "https://api.groq.com/openai/v1/chat/completions"
        headers = {"Authorization": f"Bearer {settings.GROQ_API_KEY}"}
        payload = {
            "model": model_name,
            "messages": [
                {"role": "system", "content": system or "You are Heiwa."},
                {"role": "user", "content": prompt}
            ],
            "stream": True
        }
        try:
            async with self.client.stream("POST", url, headers=headers, json=payload) as response:
                async for line in response.aiter_lines():
                    if line.startswith("data: ") and "[DONE]" not in line:
                        data = json.loads(line[6:])
                        delta = data["choices"][0]["delta"].get("content", "")
                        if delta: yield delta
        except Exception as e:
            yield f"Groq Stream failure: {e}"

    async def close(self):
        await self.client.aclose()
