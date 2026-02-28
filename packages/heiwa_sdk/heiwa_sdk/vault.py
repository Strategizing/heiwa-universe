import os
import base64
import logging
from cryptography.fernet import Fernet
from cryptography.hazmat.primitives import hashes
from cryptography.hazmat.primitives.kdf.pbkdf2 import PBKDF2HMAC

logger = logging.getLogger("SDK.Vault")

class InstanceVault:
    """
    Handles encryption/decryption for Agent Instances.
    Uses a master key to derive instance-specific keys.
    """
    def __init__(self, master_key: str = None):
        self.master_key = master_key or os.getenv("HEIWA_MASTER_KEY", "heiwa-default-safety-key")
        self._fernet = self._derive_fernet(self.master_key)

    def _derive_fernet(self, secret: str) -> Fernet:
        salt = b'heiwa-swarm-salt' # In production, this should be unique per instance
        kdf = PBKDF2HMAC(
            algorithm=hashes.SHA256(),
            length=32,
            salt=salt,
            iterations=100000,
        )
        key = base64.urlsafe_b64encode(kdf.derive(secret.encode()))
        return Fernet(key)

    def encrypt(self, data: str) -> str:
        if not data: return ""
        return self._fernet.encrypt(data.encode()).decode()

    def decrypt(self, token: str) -> str:
        if not token: return ""
        try:
            return self._fernet.decrypt(token.encode()).decode()
        except Exception as e:
            logger.error(f"Decryption failed: {e}")
            return "[DECRYPTION_ERROR]"

    @staticmethod
    def generate_master_key():
        return Fernet.generate_key().decode()