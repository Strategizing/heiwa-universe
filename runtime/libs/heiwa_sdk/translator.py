# libs/heiwa_sdk/translator.py
import re
import json

class HeiwaTranslator:
    """
    Translates Natural Language into Sovereign Directives (NATS Subjects + JSON Payloads).
    """

    def __init__(self):
        # Define deterministic command patterns (The "Reflexes")
        self.patterns = {
            r"(?i)audit": self._schema_audit,
            r"(?i)deploy": self._schema_deploy,
            r"(?i)status|report": self._schema_status,
            r"(?i)clear cache": self._schema_maintenance
        }

    def translate(self, text: str) -> dict:
        """
        Input: "System, run a deep audit on the auth module."
        Output: {
            "subject": "heiwa.tasks.audit",
            "payload": {"type": "deep", "target": "auth module", "raw_prompt": "..."}
        }
        """
        for pattern, handler in self.patterns.items():
            if re.search(pattern, text):
                directive = handler(text)
                # Ensure the original prompt is passed for context
                directive["payload"]["raw_prompt"] = text
                return directive
        
        # Fallback for unknown intents
        return {
            "subject": "heiwa.logs.unhandled",
            "payload": {"error": "Intent unrecognized", "original_text": text}
        }

    # --- Schema Definitions (The "Protocols") ---

    def _schema_audit(self, text):
        scope = "deep" if "deep" in text.lower() else "quick"
        return {
            "subject": "heiwa.tasks.audit",
            "payload": {
                "action": "audit",
                "scope": scope,
                "timestamp": "auto" 
            }
        }

    def _schema_deploy(self, text):
        target = "prod" if "prod" in text.lower() else "dev"
        return {
            "subject": "heiwa.tasks.deploy",
            "payload": {
                "action": "deploy",
                "target": target,
                "force": "force" in text.lower()
            }
        }

    def _schema_status(self, text):
        return {
            "subject": "heiwa.tasks.status",
            "payload": {"query": "system_health"}
        }

    def _schema_maintenance(self, text):
        return {
            "subject": "heiwa.tasks.maintenance",
            "payload": {"action": "clear_cache"}
        }
