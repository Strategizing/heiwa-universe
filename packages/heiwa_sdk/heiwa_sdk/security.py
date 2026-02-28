import re
import json
import hashlib
from typing import Any, List, Tuple, Optional

REDACTION_PATTERNS = [
    (re.compile(r"(gho|github_pat)_[A-Za-z0-9_]+"), r"\1_<redacted>"),
    (re.compile(r"sk-[A-Za-z0-9_-]+"), "sk-<redacted>"),
    (re.compile(r"(Authorization:\s*Bearer\s+)[^\s]+", re.I), r"\1<redacted>"),
    (re.compile(r"(nats://[^:@/\s]+:)[^@/\s]+@", re.I), r"\1<redacted>@"),
    (re.compile(r"([A-Z0-9_]*(TOKEN|SECRET|PASSWORD|KEY)[A-Z0-9_]*=)[^\s]+"), r"\1<redacted>"),
]

def redact_text(text: str) -> str:
    """Redacts sensitive patterns from a string."""
    out = text or ""
    for pattern, repl in REDACTION_PATTERNS:
        out = pattern.sub(repl, out)
    return out

def redact_any(value: Any) -> Any:
    """Recursively redacts sensitive patterns from any JSON-serializable object."""
    if isinstance(value, str):
        return redact_text(value)
    if isinstance(value, dict):
        return {str(k): redact_any(v) for k, v in value.items()}
    if isinstance(value, list):
        return [redact_any(v) for v in value]
    if isinstance(value, tuple):
        return [redact_any(v) for v in value]
    return value

def truncate_text(text: str, max_chars: int) -> str:
    """Truncates text with a standard ellipsis and note."""
    if max_chars <= 0:
        return ""
    if len(text) <= max_chars:
        return text
    return text[: max_chars - 32] + "\n...[truncated by heiwa-sdk]"

def limit_payload_size(value: Any, max_chars: int) -> Tuple[Any, bool, Optional[str]]:
    """
    Validates and potentially truncates a payload.
    Returns (modified_value, was_truncated, sha256_hash).
    """
    def _json_default(obj):
        return str(obj)

    try:
        raw = json.dumps(value, default=_json_default, separators=(",", ":"))
    except Exception:
        raw = str(value)
    
    payload_sha256 = hashlib.sha256(raw.encode("utf-8", "ignore")).hexdigest()
    
    if len(raw) <= max_chars:
        return value, False, payload_sha256
        
    preview = truncate_text(raw, max_chars)
    return (
        {
            "_truncated": True,
            "_preview": preview,
            "_original_sha256": payload_sha256,
            "_original_size_chars": len(raw),
        },
        True,
        payload_sha256,
    )
