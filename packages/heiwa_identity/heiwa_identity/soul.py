from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SOUL_PATH = ROOT / "config/identities/soul/core.md"

def get_soul() -> str:
    """Read the core soul manifest of Heiwa."""
    if not SOUL_PATH.exists():
        return "Helpful and opinionated AI entity."
    return SOUL_PATH.read_text(encoding="utf-8")

def get_identity_meta() -> str:
    """Read the current persona identity metadata."""
    meta_path = ROOT / "config/identities/persona/identity.md"
    if not meta_path.exists():
        return "Identity undefined."
    return meta_path.read_text(encoding="utf-8")
