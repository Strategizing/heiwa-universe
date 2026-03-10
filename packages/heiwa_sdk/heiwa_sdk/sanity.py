# libs/heiwa_sdk/sanity.py
"""
Heiwa Sanity Layer: Prevents credential leakage to Discord/Logs.
"""
import re

class HeiwaSanity:
    """Prevents agents from accidentally posting secrets to the Boardroom."""
    
    SECRET_PATTERNS = [
        r"OT[a-zA-Z0-9\._\-]{50,}",       # Discord Token
        r"sk-[a-zA-Z0-9]{32,}",            # OpenAI / Moltbook Keys
        r"nats://[^@]+:[^@]+@",            # NATS Auth strings
        r"ghp_[a-zA-Z0-9]{36,}",           # GitHub PAT
        r"ghs_[a-zA-Z0-9]{36,}",           # GitHub App Token
        r"xoxb-[a-zA-Z0-9\-]+",            # Slack Bot Token
        r"AKIA[0-9A-Z]{16}",               # AWS Access Key
        r"-----BEGIN (RSA |EC )?PRIVATE KEY-----", # Private Keys
    ]

    @classmethod
    def redact(cls, text: str) -> str:
        """Scans text and replaces any matching secrets with [REDACTED]."""
        for pattern in cls.SECRET_PATTERNS:
            text = re.sub(pattern, "[REDACTED_SENSITIVE_KEY]", text, flags=re.IGNORECASE)
        return text

    @classmethod
    def is_safe(cls, text: str) -> bool:
        """Returns True if no secrets are detected."""
        for pattern in cls.SECRET_PATTERNS:
            if re.search(pattern, text, flags=re.IGNORECASE):
                return False
        return True