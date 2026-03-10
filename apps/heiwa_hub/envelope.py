def extract_auth_token(envelope: dict) -> str | None:
    """Extract auth token from various envelope structures."""
    return envelope.get("auth_token") or envelope.get("data", {}).get("auth_token")

def extract_payload(envelope: dict) -> dict:
    """Extract inner payload from the envelope."""
    payload = envelope.get("data", envelope)
    # Ensure it's a dict
    if not isinstance(payload, dict):
        return {"raw_text": str(payload)}
    return payload

def normalize_sender(envelope: dict) -> str:
    """Extract canonical sender ID."""
    return envelope.get("sender_id") or envelope.get("source") or "anonymous"
