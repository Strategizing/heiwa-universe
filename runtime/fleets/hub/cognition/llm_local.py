"""
Heiwa LLM Engine — Tiered Multi-Provider Router

Tier 1: Node Ollama  (Macbook M4/Workstation RTX 3060 — free, unlimited)
Tier 2: Gemini Flash (Google AI Pro plan — fast, cheap)
Tier 3: Gemini Pro   (Google AI Pro plan — heavy reasoning)
Tier 4: ChatGPT Plus (OpenAI — highest quality, most expensive)

Routing logic:
  LOW complexity  → Tier 1 (Ollama) → fallback Tier 2
  MED complexity  → Tier 2 (Gemini Flash) → fallback Tier 1
  HIGH complexity → Tier 3 (Gemini Pro) → fallback Tier 4 → fallback Tier 1
"""
from __future__ import annotations

import json
import logging
import os
from dataclasses import dataclass
from typing import Any, Optional

import requests
from tenacity import (
    retry,
    stop_after_attempt,
    wait_exponential,
    retry_if_exception_type,
)

try:
    from libs.heiwa_sdk.heiwa_net import HeiwaNetProxy
    _NET_PROXY = HeiwaNetProxy(origin_surface="runtime", agent_id="llm-engine")
except ImportError:
    _NET_PROXY = None

logger = logging.getLogger("LLMEngine")


class LLMPolicyError(RuntimeError):
    """Raised when runtime config violates policy."""


@dataclass
class LLMResult:
    text: str
    provider: str
    model: str
    tier: int


class LocalLLMEngine:
    """
    Tiered multi-provider LLM engine.

    Routes requests through the cheapest viable provider first,
    escalating only when lower tiers are unavailable or the task
    demands higher capability.
    """

    def __init__(self) -> None:
        self.host_runtime = self._detect_host_runtime()

        # --- Ollama (Tier 1: Free, local inference) ---
        self.ollama_url = os.getenv(
            "HEIWA_OLLAMA_URL", "http://127.0.0.1:11434"
        ).rstrip("/")
        self.ollama_model = os.getenv("HEIWA_OLLAMA_MODEL", "qwen2.5-coder:7b")
        self.ollama_timeout = float(os.getenv("HEIWA_OLLAMA_TIMEOUT_SEC", "60"))
        self.ollama_enabled_env = os.getenv("HEIWA_ENABLE_OLLAMA", "true").strip().lower() == "true"
        self.ollama_allowed_by_runtime = self._runtime_allows_ollama(self.host_runtime)
        self.ollama_enabled = self.ollama_enabled_env and self.ollama_allowed_by_runtime
        if self.ollama_enabled_env and not self.ollama_allowed_by_runtime:
            logger.warning(
                "Ollama disabled by runtime policy (host_runtime=%s). "
                "Railway/cloud executors must use remote providers (e.g., Gemini via Google API key).",
                self.host_runtime,
            )

        # --- Gemini (Tier 2/3: Google AI Pro plan) ---
        self.gemini_key = os.getenv("GEMINI_API_KEY")
        self.gemini_flash_model = os.getenv(
            "HEIWA_GEMINI_FLASH_MODEL", "gemini-2.5-flash"
        )
        self.gemini_pro_model = os.getenv(
            "HEIWA_GEMINI_PRO_MODEL", "gemini-2.5-pro"
        )
        self.gemini_timeout = float(os.getenv("HEIWA_GEMINI_TIMEOUT_SEC", "30"))

        # --- OpenAI / ChatGPT Plus (Tier 4: Highest quality) ---
        self.openai_key = os.getenv("OPENAI_API_KEY")
        self.openai_model = os.getenv("HEIWA_OPENAI_MODEL", "gpt-4o")
        self.openai_timeout = float(os.getenv("HEIWA_OPENAI_TIMEOUT_SEC", "45"))

        # --- Rate limiting (optional Redis) ---
        self._redis = None
        redis_url = os.getenv("REDIS_URL")
        if redis_url:
            try:
                import redis
                self._redis = redis.Redis.from_url(redis_url, decode_responses=True)
                self._redis.ping()
            except Exception:
                logger.warning("Redis unavailable — rate limits will not be tracked.")
                self._redis = None

        logger.info(
            "LLMEngine initialized | host_runtime=%s ollama=%s gemini=%s openai=%s",
            self.host_runtime,
            "ON" if self._ollama_available(runtime=self.host_runtime) else "OFF",
            "ON" if self.gemini_key else "OFF",
            "ON" if self.openai_key else "OFF",
        )

    # ------------------------------------------------------------------ #
    #  Availability checks                                                 #
    # ------------------------------------------------------------------ #

    @staticmethod
    def _detect_host_runtime() -> str:
        explicit = str(os.getenv("HEIWA_EXECUTOR_RUNTIME", "")).strip().lower()
        if explicit:
            return explicit
        if os.getenv("RAILWAY_ENVIRONMENT") or os.getenv("RAILWAY_ENVIRONMENT_NAME") or os.getenv("RAILWAY_PROJECT_ID"):
            return "railway"
        if os.getenv("HEIWA_LLM_MODE", "").strip().lower() == "local_only":
            return "macbook"
        return "auto"

    @staticmethod
    def _normalize_runtime(runtime: str | None) -> str:
        value = str(runtime or "auto").strip().lower()
        return value or "auto"

    @staticmethod
    def _runtime_allows_ollama(runtime: str | None) -> bool:
        value = LocalLLMEngine._normalize_runtime(runtime)
        return value not in {"railway", "cloud"}

    def _effective_runtime(self, runtime: str = "auto") -> str:
        value = self._normalize_runtime(runtime)
        return self.host_runtime if value == "auto" else value

    def _ollama_available(self, runtime: str = "auto") -> bool:
        effective_runtime = self._effective_runtime(runtime)
        if not self.ollama_enabled:
            return False
        if not self._runtime_allows_ollama(effective_runtime):
            return False
        try:
            if _NET_PROXY:
                resp = _NET_PROXY.get(
                    f"{self.ollama_url}/api/tags",
                    purpose="ollama availability check",
                    purpose_class="health_check",
                    timeout=int(self.ollama_timeout),
                )
            else:
                resp = requests.get(
                    f"{self.ollama_url}/api/tags", timeout=self.ollama_timeout
                )
            return resp.status_code == 200
        except (requests.RequestException, PermissionError):
            return False

    def is_available(self, runtime: str = "auto") -> bool:
        """Returns True if at least one provider is reachable."""
        if self._ollama_available(runtime=runtime):
            return True
        if self.gemini_key:
            return True
        if self.openai_key:
            return True
        return False

    # ------------------------------------------------------------------ #
    #  Provider calls                                                      #
    # ------------------------------------------------------------------ #

    def _call_ollama(
        self, prompt: str, system: Optional[str] = None
    ) -> LLMResult:
        import asyncio
        import threading
        from nats.aio.client import Client as NATS
        
        nats_url = os.getenv("NATS_URL", "nats://localhost:4222")
        result_container = {}

        async def fetch_from_macbook():
            nc = NATS()
            try:
                await nc.connect(nats_url, connect_timeout=3.0)
                request_payload = {
                    "prompt": prompt,
                    "system": system,
                    "complexity": "high" if "deepseek" in self.ollama_model else "low"
                }
                msg = await nc.request("heiwa.inference.request", json.dumps(request_payload).encode(), timeout=10.0)
                data = json.loads(msg.data.decode())
                await nc.close()
                
                if "error" in data:
                    result_container["error"] = Exception(data["error"])
                else:
                    result_container["text"] = data.get("text", "")
            except Exception as e:
                if nc.is_connected:
                    await nc.close()
                result_container["error"] = e

        def run_in_thread():
            loop = asyncio.new_event_loop()
            asyncio.set_event_loop(loop)
            try:
                loop.run_until_complete(fetch_from_macbook())
            finally:
                loop.close()

        t = threading.Thread(target=run_in_thread)
        t.start()
        t.join(timeout=15.0)

        if t.is_alive():
            raise requests.exceptions.HTTPError("Macbook Node timeout (15s). NATS request stalled.")

        if "error" in result_container:
            logger.warning(f"Failed to reach Macbook GPU Node via NATS: {result_container['error']}. Falling back...")
            raise requests.exceptions.HTTPError(f"Macbook Node Unavailable: {result_container['error']}")

        text = result_container.get("text", "")
        return LLMResult(
            text=text, provider="ollama-macbook-gpu", model=self.ollama_model, tier=1
        )

    @retry(
        stop=stop_after_attempt(2),
        wait=wait_exponential(multiplier=2, min=2, max=30),
        retry=retry_if_exception_type(requests.exceptions.HTTPError),
    )
    def _call_gemini(
        self,
        prompt: str,
        model_name: str,
        tier: int,
        system: Optional[str] = None,
    ) -> LLMResult:
        url = (
            f"https://generativelanguage.googleapis.com/v1beta/models/"
            f"{model_name}:generateContent?key={self.gemini_key}"
        )
        payload: dict[str, Any] = {
            "contents": [{"parts": [{"text": prompt}]}],
            "generationConfig": {"temperature": 0.2},
        }
        if system:
            payload["systemInstruction"] = {"parts": [{"text": system}]}

        if _NET_PROXY:
            resp = _NET_PROXY.post(
                url, purpose=f"gemini {model_name} inference",
                purpose_class="model_inference", json=payload,
                timeout=int(self.gemini_timeout),
            )
        else:
            resp = requests.post(url, json=payload, timeout=self.gemini_timeout)
        if resp.status_code == 429:
            logger.warning("Gemini 429 on %s — backing off", model_name)
            resp.raise_for_status()
        resp.raise_for_status()

        data = resp.json()
        text = ""
        if "candidates" in data and data["candidates"]:
            text = str(
                data["candidates"][0]["content"]["parts"][0]["text"]
            ).strip()
        return LLMResult(text=text, provider="gemini", model=model_name, tier=tier)

    @retry(
        stop=stop_after_attempt(2),
        wait=wait_exponential(multiplier=2, min=2, max=30),
        retry=retry_if_exception_type(requests.exceptions.HTTPError),
    )
    def _call_openai(
        self, prompt: str, system: Optional[str] = None
    ) -> LLMResult:
        url = "https://api.openai.com/v1/chat/completions"
        messages = []
        if system:
            messages.append({"role": "system", "content": system})
        messages.append({"role": "user", "content": prompt})

        payload = {
            "model": self.openai_model,
            "messages": messages,
            "temperature": 0.2,
        }
        headers = {
            "Authorization": f"Bearer {self.openai_key}",
            "Content-Type": "application/json",
        }
        if _NET_PROXY:
            resp = _NET_PROXY.post(
                url, purpose="openai inference",
                purpose_class="model_inference",
                json=payload, headers=headers,
                timeout=int(self.openai_timeout),
            )
        else:
            resp = requests.post(
                url, json=payload, headers=headers, timeout=self.openai_timeout
            )
        if resp.status_code == 429:
            logger.warning("OpenAI 429 — backing off")
            resp.raise_for_status()
        resp.raise_for_status()

        data = resp.json()
        text = data["choices"][0]["message"]["content"].strip()
        return LLMResult(
            text=text, provider="openai", model=self.openai_model, tier=4
        )

    # ------------------------------------------------------------------ #
    #  Tiered routing                                                      #
    # ------------------------------------------------------------------ #

    def _tier_chain(self, complexity: str, runtime: str = "auto") -> list[str]:
        """Return ordered list of providers to try for a given complexity."""
        effective_runtime = self._effective_runtime(runtime)
        if complexity == "low":
            chain = ["ollama", "gemini_flash"]
        elif complexity == "medium":
            chain = ["gemini_flash", "ollama", "gemini_pro"]
        else:  # high
            chain = ["gemini_pro", "openai", "ollama"]
        if not self._runtime_allows_ollama(effective_runtime):
            chain = [provider for provider in chain if provider != "ollama"]
        return chain

    def _try_provider(
        self,
        provider: str,
        prompt: str,
        system: Optional[str],
        runtime: str = "auto",
    ) -> Optional[LLMResult]:
        """Attempt a single provider. Returns None on failure."""
        try:
            if provider == "ollama" and self._ollama_available(runtime=runtime):
                return self._call_ollama(prompt, system)
            elif provider == "gemini_flash" and self.gemini_key:
                return self._call_gemini(
                    prompt, self.gemini_flash_model, tier=2, system=system
                )
            elif provider == "gemini_pro" and self.gemini_key:
                return self._call_gemini(
                    prompt, self.gemini_pro_model, tier=3, system=system
                )
            elif provider == "openai" and self.openai_key:
                return self._call_openai(prompt, system)
        except Exception as e:
            logger.warning("Provider %s failed: %s", provider, e)
        return None

    def generate(
        self,
        prompt: str,
        complexity: str = "low",
        system: Optional[str] = None,
        # Legacy param kept for backward compat with existing callers
        runtime: str = "auto",
    ) -> str:
        """
        Generate text using tiered provider routing.

        complexity: "low" | "medium" | "high"
        """
        chain = self._tier_chain(complexity, runtime=runtime)
        for provider in chain:
            result = self._try_provider(provider, prompt, system, runtime=runtime)
            if result and result.text:
                logger.info(
                    "LLM response via %s/%s (tier %d) [runtime=%s]",
                    result.provider,
                    result.model,
                    result.tier,
                    self._effective_runtime(runtime),
                )
                return result.text

        logger.error("All LLM providers exhausted for prompt: %s...", prompt[:60])
        return ""

    def generate_json(
        self,
        prompt: str,
        complexity: str = "low",
        system: Optional[str] = None,
        runtime: str = "auto",
    ) -> dict[str, Any]:
        """Generate and parse JSON from LLM response."""
        text = self.generate(prompt=prompt, complexity=complexity, system=system, runtime=runtime)
        if not text:
            return {}

        text = text.strip()
        if text.startswith("```"):
            text = text.strip("`").replace("json", "", 1).strip()

        try:
            return json.loads(text)
        except json.JSONDecodeError:
            start = text.find("{")
            end = text.rfind("}")
            if start != -1 and end != -1 and end > start:
                try:
                    return json.loads(text[start : end + 1])
                except json.JSONDecodeError:
                    return {}
        return {}
