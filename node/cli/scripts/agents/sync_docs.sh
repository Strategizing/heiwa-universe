#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT"

mkdir -p docs/agents/{codex,opencode,openclaw,ollama}

sha256_file() {
  python3 - "$1" <<'PY'
import hashlib,sys
p=sys.argv[1]
b=open(p,'rb').read()
print(hashlib.sha256(b).hexdigest())
PY
}

# Pin versions you actually run (update explicitly)
echo "codex-cli: tracked via local install" > docs/agents/codex/PINNED_VERSION.md
echo "opencode: v0.8.4-alpha" > docs/agents/opencode/PINNED_VERSION.md
echo "openclaw: DISABLED (pending audit)" > docs/agents/openclaw/PINNED_VERSION.md
echo "ollama: local daemon" > docs/agents/ollama/PINNED_VERSION.md

# Fetch upstream canon
curl -fsSL https://raw.githubusercontent.com/openai/codex/main/README.md > docs/agents/codex/UPSTREAM.md
curl -fsSL https://raw.githubusercontent.com/opencode-ai/opencode/main/README.md > docs/agents/opencode/UPSTREAM.md
curl -fsSL https://raw.githubusercontent.com/openclaw/openclaw/main/README.md > docs/agents/openclaw/UPSTREAM.md

# Heiwa overlay (security)
cat > docs/agents/openclaw/SECURITY_POLICY.md <<'EOF'
# OpenClaw Security Policy (Heiwa)
- Third-party skills: BLOCKED
- Only core execution modules allowed
- OpenClaw remains DISABLED until version pinned + code audited
EOF

# Manifest
MAN="docs/agents/MANIFEST_SHA256.md"
{
  echo "# Docs Canon Manifest (SHA256)"
  echo
  for f in docs/agents/**/PINNED_VERSION.md docs/agents/**/UPSTREAM.md docs/agents/**/SECURITY_POLICY.md; do
    [ -f "$f" ] || continue
    python3 - "$f" <<'PY'
import hashlib,sys
p=sys.argv[1]
b=open(p,"rb").read()
print(f"- `{p}`  `{hashlib.sha256(b).hexdigest()}`")
PY
  done
} > "$MAN"

echo "[OK] Synced canon docs + wrote $MAN"
