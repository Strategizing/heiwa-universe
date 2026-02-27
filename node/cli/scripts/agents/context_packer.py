#!/usr/bin/env python3
# cli/scripts/agents/context_packer.py
from __future__ import annotations

import hashlib
import os
import re
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Iterable, List, Tuple

import yaml

DEFAULT_OUT = "runtime/context/CONTEXT_PACK.md"

# Hard deny patterns (avoid secrets)
DENY_GLOBS = [
    "**/.env",
    "**/.env.*",
    "**/*.pem",
    "**/*id_rsa*",
    "**/*id_ed25519*",
    "**/node_modules/**",
    "**/.git/**",
    "**/runtime/**",
]

ALLOW_DEFAULT = [
    "docs/ARCHITECTURE.md",
    "docs/OPERATING_DOCTRINE.md",
    "docs/SECURITY_MODEL.md",
    "docs/agents/SECURITY_MODEL.md",
    "docs/agents/**/PINNED_VERSION.md",
    "docs/agents/**/UPSTREAM.md",
    "docs/agents/**/SECURITY_POLICY.md",
]

@dataclass(frozen=True)
class Limits:
    max_files: int
    max_file_bytes_each: int
    max_total_bytes: int

def sha256_bytes(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()

def read_text_bounded(path: Path, limit: int) -> Tuple[str, int, str]:
    raw = path.read_bytes()
    h = sha256_bytes(raw)
    if len(raw) > limit:
        raw = raw[:limit]
    # Conservative decode
    text = raw.decode("utf-8", errors="replace")
    return text, len(raw), h

def glob_many(root: Path, patterns: Iterable[str]) -> List[Path]:
    out: List[Path] = []
    for pat in patterns:
        out.extend(root.glob(pat))
    # unique + stable order
    seen = set()
    uniq = []
    for p in sorted(out, key=lambda x: str(x)):
        if p.is_file() and p not in seen:
            seen.add(p)
            uniq.append(p)
    return uniq

def matches_any(path: Path, root: Path, globs: List[str]) -> bool:
    rel = path.relative_to(root)
    for g in globs:
        if rel.match(g):
            return True
    return False

def sanitize_title(s: str) -> str:
    s = re.sub(r"[^\w\-\.\s/]", "", s)
    return s.strip()

def load_limits(cfg_path: Path) -> Limits:
    cfg = yaml.safe_load(cfg_path.read_text(encoding="utf-8"))
    b = cfg["budgets"]
    return Limits(
        max_files=int(b["max_context_files"]),
        max_file_bytes_each=int(b["max_file_bytes_each"]),
        max_total_bytes=int(b["max_total_context_bytes"]),
    )

def main() -> int:
    repo = Path(os.environ.get("HEIWA_WORKSPACE_ROOT", ".")).resolve()
    cfg_path = repo / "config" / "agents.yaml"
    out_path = repo / os.environ.get("HEIWA_CONTEXT_OUT", DEFAULT_OUT)
    out_path.parent.mkdir(parents=True, exist_ok=True)

    if not cfg_path.exists():
        print(f"[ERR] Missing config: {cfg_path}", file=sys.stderr)
        return 2

    limits = load_limits(cfg_path)

    allow = list(ALLOW_DEFAULT)
    # Optional extra allow patterns passed via CLI args
    allow.extend(sys.argv[1:])

    candidates = glob_many(repo, allow)
    # Deny filter
    files = [p for p in candidates if not matches_any(p, repo, DENY_GLOBS)]

    # Enforce file count
    files = files[: limits.max_files]

    total = 0
    manifest = []
    blocks = []

    for p in files:
        rel = p.relative_to(repo)
        text, used_bytes, h = read_text_bounded(p, limits.max_file_bytes_each)

        if total + used_bytes > limits.max_total_bytes:
            break

        total += used_bytes
        manifest.append((str(rel), used_bytes, h))

        blocks.append(
            "\n".join(
                [
                    f"## {sanitize_title(str(rel))}",
                    f"- bytes_used: {used_bytes}",
                    f"- sha256: {h}",
                    "",
                    "```",
                    text.rstrip(),
                    "```",
                    "",
                ]
            )
        )

    header = "\n".join(
        [
            "# HEIWA CONTEXT PACK (Deterministic)",
            "",
            f"- repo: {repo}",
            f"- files_included: {len(manifest)}",
            f"- bytes_total: {total}",
            "",
            "## Manifest",
            "",
        ]
    )

    man_lines = ["| path | bytes_used | sha256 |", "|---|---:|---|"]
    for rel, used, h in manifest:
        man_lines.append(f"| `{rel}` | {used} | `{h}` |")

    content = "\n".join([header, "\n".join(man_lines), "", *blocks])
    out_path.write_text(content, encoding="utf-8")

    print(f"[OK] Wrote {out_path} ({len(manifest)} files, {total} bytes)")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
