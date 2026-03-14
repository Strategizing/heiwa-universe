"""
Rate-group-aware ledger for OAuth CLI tool routing.

Tracks usage per rate_group in sliding windows. Heiwa routes through
subscription-tier CLI tools (Claude Code, Gemini CLI, Codex, Antigravity,
Ollama) — rate limits are "turns per window", not tokens per minute.

The ledger is process-local. On Railway, Spine and Executor share it
via import. Remote workers track their own local groups.
"""

from __future__ import annotations

import json
import logging
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any

logger = logging.getLogger("SDK.RateLedger")


@dataclass(slots=True)
class RateGroupConfig:
    """Rate limit configuration for a single provider group."""
    group: str
    max_turns: int          # max requests per window
    window_sec: float       # sliding window duration in seconds
    cooldown_sec: float     # backoff after hitting limit
    unlimited: bool = False # local groups are unlimited


@dataclass(slots=True)
class _GroupState:
    """Mutable per-group tracking state."""
    timestamps: list[float] = field(default_factory=list)
    cooldown_until: float = 0.0


# Default rate limits based on Devon's subscriptions:
#   Claude Pro: ~45 Opus messages per 5 hours
#   ChatGPT Plus Codex: ~30 turns per hour (conservative)
#   Google AI Pro (Gemini CLI): generous, ~60/hr
#   Google AI Pro (Antigravity): separate rate group, ~40/hr
#   SiliconFlow/Cerebras/etc: free tiers, ~20/hr
#   Local Ollama/vLLM: unlimited
_DEFAULT_LIMITS: dict[str, dict[str, Any]] = {
    "local_ollama": {"max_turns": 0, "window_sec": 0, "cooldown_sec": 0, "unlimited": True},
    "local_runtime": {"max_turns": 0, "window_sec": 0, "cooldown_sec": 0, "unlimited": True},
    "local_vllm": {"max_turns": 0, "window_sec": 0, "cooldown_sec": 0, "unlimited": True},
    "local_litellm": {"max_turns": 0, "window_sec": 0, "cooldown_sec": 0, "unlimited": True},
    "deterministic_ops": {"max_turns": 0, "window_sec": 0, "cooldown_sec": 0, "unlimited": True},
    "claude_code": {"max_turns": 40, "window_sec": 18000, "cooldown_sec": 300},
    "openai_codex": {"max_turns": 25, "window_sec": 3600, "cooldown_sec": 120},
    "google_gemini_cli": {"max_turns": 50, "window_sec": 3600, "cooldown_sec": 60},
    "google_antigravity": {"max_turns": 35, "window_sec": 3600, "cooldown_sec": 60},
    "siliconflow": {"max_turns": 15, "window_sec": 3600, "cooldown_sec": 120},
    "cerebras": {"max_turns": 20, "window_sec": 3600, "cooldown_sec": 90},
    "openrouter": {"max_turns": 10, "window_sec": 3600, "cooldown_sec": 180},
    "groq": {"max_turns": 20, "window_sec": 3600, "cooldown_sec": 90},
}


class RateGroupLedger:
    """
    Process-local rate-group tracker with sliding window enforcement.

    Usage:
        ledger = get_rate_ledger()
        if ledger.has_capacity("claude_code"):
            ledger.record("claude_code")
            # ... execute via Claude Code
        else:
            # cascade to next group
    """

    def __init__(self, router_path: Path | None = None) -> None:
        self._lock = threading.Lock()
        self._groups: dict[str, RateGroupConfig] = {}
        self._state: dict[str, _GroupState] = {}
        self._load_limits(router_path)

    def _load_limits(self, router_path: Path | None) -> None:
        """Load rate limits from ai_router.json, falling back to defaults."""
        overrides: dict[str, dict[str, Any]] = {}
        if router_path and router_path.exists():
            try:
                config = json.loads(router_path.read_text(encoding="utf-8"))
                overrides = config.get("rate_limits", {})
            except Exception:
                pass

        for group, defaults in _DEFAULT_LIMITS.items():
            merged = {**defaults, **(overrides.get(group, {}))}
            self._groups[group] = RateGroupConfig(
                group=group,
                max_turns=int(merged.get("max_turns", 0)),
                window_sec=float(merged.get("window_sec", 0)),
                cooldown_sec=float(merged.get("cooldown_sec", 0)),
                unlimited=bool(merged.get("unlimited", False)),
            )
            self._state[group] = _GroupState()

        # Also load any overrides for groups not in defaults
        for group, cfg in overrides.items():
            if group not in self._groups:
                self._groups[group] = RateGroupConfig(
                    group=group,
                    max_turns=int(cfg.get("max_turns", 10)),
                    window_sec=float(cfg.get("window_sec", 3600)),
                    cooldown_sec=float(cfg.get("cooldown_sec", 120)),
                    unlimited=bool(cfg.get("unlimited", False)),
                )
                self._state[group] = _GroupState()

    def _ensure_group(self, group: str) -> RateGroupConfig:
        if group not in self._groups:
            self._groups[group] = RateGroupConfig(
                group=group, max_turns=10, window_sec=3600, cooldown_sec=120,
            )
            self._state[group] = _GroupState()
        return self._groups[group]

    def _prune(self, group: str, now: float) -> None:
        cfg = self._groups[group]
        state = self._state[group]
        if cfg.unlimited:
            return
        cutoff = now - cfg.window_sec
        state.timestamps = [t for t in state.timestamps if t > cutoff]

    def has_capacity(self, group: str) -> bool:
        """Check if a rate group has remaining capacity."""
        with self._lock:
            cfg = self._ensure_group(group)
            if cfg.unlimited:
                return True
            now = time.time()
            state = self._state[group]
            if now < state.cooldown_until:
                return False
            self._prune(group, now)
            return len(state.timestamps) < cfg.max_turns

    def record(self, group: str) -> None:
        """Record a usage event for a rate group."""
        with self._lock:
            cfg = self._ensure_group(group)
            if cfg.unlimited:
                return
            now = time.time()
            state = self._state[group]
            self._prune(group, now)
            state.timestamps.append(now)
            if len(state.timestamps) >= cfg.max_turns:
                state.cooldown_until = now + cfg.cooldown_sec
                logger.info(
                    "Rate group '%s' hit limit (%d/%d). Cooldown %.0fs.",
                    group, len(state.timestamps), cfg.max_turns, cfg.cooldown_sec,
                )

    def record_throttle(self, group: str) -> None:
        """Record an external throttle signal (e.g., 429 from provider)."""
        with self._lock:
            cfg = self._ensure_group(group)
            state = self._state[group]
            state.cooldown_until = time.time() + cfg.cooldown_sec
            logger.warning("Rate group '%s' externally throttled. Cooldown %.0fs.", group, cfg.cooldown_sec)

    def remaining(self, group: str) -> int | None:
        """Return remaining turns for a group, or None if unlimited."""
        with self._lock:
            cfg = self._ensure_group(group)
            if cfg.unlimited:
                return None
            now = time.time()
            self._prune(group, now)
            state = self._state[group]
            return max(0, cfg.max_turns - len(state.timestamps))

    def status(self) -> dict[str, dict[str, Any]]:
        """Return status for all tracked groups."""
        with self._lock:
            now = time.time()
            result = {}
            for group, cfg in self._groups.items():
                if cfg.unlimited:
                    result[group] = {"unlimited": True, "available": True}
                    continue
                self._prune(group, now)
                state = self._state[group]
                used = len(state.timestamps)
                available = now >= state.cooldown_until and used < cfg.max_turns
                result[group] = {
                    "used": used,
                    "max": cfg.max_turns,
                    "window_sec": cfg.window_sec,
                    "available": available,
                    "cooldown_remaining": max(0, state.cooldown_until - now),
                }
            return result


# Module-level singleton
_ledger: RateGroupLedger | None = None
_ledger_lock = threading.Lock()


def get_rate_ledger(router_path: Path | None = None) -> RateGroupLedger:
    """Return the process-local rate ledger singleton."""
    global _ledger
    if _ledger is None:
        with _ledger_lock:
            if _ledger is None:
                if router_path is None:
                    root = Path(__file__).resolve().parents[3]
                    router_path = root / "config" / "swarm" / "ai_router.json"
                _ledger = RateGroupLedger(router_path)
    return _ledger
