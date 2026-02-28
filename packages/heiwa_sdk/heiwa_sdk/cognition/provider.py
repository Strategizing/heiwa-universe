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

logger = logging.getLogger("SDK.Cognition.Provider")

@dataclass
class TokenUsage:
    prompt_tokens: int = 0
    completion_tokens: int = 0
    total_tokens: int = 0

class LLMProvider:
    """
    SOTA Async LLM Provider.
    Handles streaming, tiered routing, and multi-provider failover.
    """
    
    def __init__(self):
        self.identity = load_node_identity()
        self.client = httpx.AsyncClient(timeout=60.0)

    async def generate_stream(self, 
                               prompt: str, 
                               model: str, 
                               system: Optional[str] = None) -> AsyncGenerator[str, None]:
        """Stream tokens with multi-provider failover."""
        providers = []
        if "google/" in model:
            providers = ["gemini", "anthropic", "groq", "ollama"]
        elif "anthropic/" in model:
            providers = ["anthropic", "gemini", "groq", "ollama"]
        elif "ollama/" in model:
            providers = ["ollama", "gemini", "anthropic"]
        else:
            providers = ["gemini", "anthropic", "groq", "ollama"]

        last_error = ""
        for provider in providers:
            try:
                success = False
                if provider == "gemini":
                    async for chunk in self._stream_gemini(prompt, "gemini-2.0-flash", system):
                        if "Error:" in chunk or "failure:" in chunk:
                            last_error = chunk
                            break
                        yield chunk
                        success = True
                elif provider == "anthropic":
                    async for chunk in self._stream_anthropic(prompt, "claude-3-5-sonnet-latest", system):
                        if "Error:" in chunk or "failure:" in chunk:
                            last_error = chunk
                            break
                        yield chunk
                        success = True
                elif provider == "groq":
                    async for chunk in self._stream_groq(prompt, "llama-3.3-70b-versatile", system):
                        if "Error:" in chunk or "failure:" in chunk:
                            last_error = chunk
                            break
                        yield chunk
                        success = True
                elif provider == "ollama":
                    async for chunk in self._stream_ollama(prompt, "qwen2.5-coder:7b", system):
                        if "Error:" in chunk or "failure:" in chunk:
                            last_error = chunk
                            break
                        yield chunk
                        success = True
                
                if success:
                    return # Successfully generated from this provider
            except Exception as e:
                last_error = str(e)
                continue

        yield f"❌ [ALL PROVIDERS EXHAUSTED] Last error: {last_error}"

    async def _stream_gemini(self, prompt: str, model_name: str, system: Optional[str]) -> AsyncGenerator[str, None]:
        api_key = os.getenv("GEMINI_API_KEY", "")
        if not api_key:
            yield "❌ [ERROR] GEMINI_API_KEY is not set."
            return
        url = f"https://generativelanguage.googleapis.com/v1beta/models/{model_name}:streamGenerateContent?key={api_key}"
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

    async def _stream_anthropic(self, prompt: str, model_name: str, system: Optional[str]) -> AsyncGenerator[str, None]:
        url = "https://api.anthropic.com/v1/messages"
        headers = {
            "x-api-key": os.getenv("ANTHROPIC_API_KEY", ""),
            "anthropic-version": "2023-06-01",
            "content-type": "application/json",
            "anthropic-beta": "messages-2023-12-15"
        }
        payload = {
            "model": model_name,
            "max_tokens": 4096,
            "messages": [{"role": "user", "content": prompt}],
            "stream": True
        }
        if system:
            payload["system"] = system

        try:
            async with self.client.stream("POST", url, headers=headers, json=payload) as response:
                async for line in response.aiter_lines():
                    if line.startswith("data: "):
                        data = json.loads(line[6:])
                        if data["type"] == "content_block_delta":
                            yield data["delta"].get("text", "")
        except Exception as e:
            yield f"Anthropic Stream failure: {e}"

    async def close(self):
        await self.client.aclose()
