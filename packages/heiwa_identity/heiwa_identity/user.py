from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
USER_PATH = ROOT / "config/identities/persona/user.md"

def get_user_context() -> str:
    """Read the context about the human operator."""
    if not USER_PATH.exists():
        return "Operator context undefined."
    return USER_PATH.read_text(encoding="utf-8")
