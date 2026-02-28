import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]

REPLACEMENTS = {
    "apps/heiwa_hub": "apps/heiwa_hub",
    "apps/heiwa_cli": "apps/heiwa_cli",
    "packages/heiwa_sdk/heiwa_sdk": "packages/heiwa_sdk/heiwa_sdk",
    "packages/heiwa_protocol/heiwa_protocol": "packages/heiwa_protocol/heiwa_protocol",
    "from heiwa_protocol.protocol": "from heiwa_protocol.protocol",
    "from heiwa_sdk": "from heiwa_sdk",
    "import heiwa_sdk": "import heiwa_sdk",
    "apps.heiwa_hub": "apps.heiwa_hub",
    "config/swarm/swarm.json": "config/swarm/swarm.json",
    "config/identities/profiles.json": "config/identities/profiles.json",
}

def refactor_file(file_path):
    try:
        content = file_path.read_text(errors="ignore")
        new_content = content
        for old, new in REPLACEMENTS.items():
            new_content = new_content.replace(old, new)
        
        # SOTA Path Normalization
        if 'packages/heiwa_sdk' in new_content:
            new_content = new_content.replace('packages/heiwa_sdk', 'packages/heiwa_sdk')
        if 'packages/heiwa_protocol/heiwa_protocol' in new_content:
            new_content = new_content.replace('packages/heiwa_protocol/heiwa_protocol', 'packages/heiwa_protocol')
        if 'packages/heiwa-ui' in new_content:
            new_content = new_content.replace('packages/heiwa-ui', 'packages/heiwa-ui')
        if 'packages/heiwa-identity' in new_content:
            new_content = new_content.replace('packages/heiwa-identity', 'packages/heiwa-identity')
        
        # Fix sys.path insertions to be enterprise-ready
        # We want to add the root of the package/app collections
        if 'sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))' in new_content:
            pass # already good
        elif 'sys.path.insert(0, str(RUNTIME_ROOT))' in new_content:
            new_content = new_content.replace('sys.path.insert(0, str(RUNTIME_ROOT))', 'sys.path.insert(0, str(ROOT / "packages/heiwa_sdk"))\n    sys.path.insert(0, str(ROOT / "packages/heiwa_protocol"))')

        if new_content != content:
            file_path.write_text(new_content)
            print(f"✅ Refactored: {file_path.relative_to(ROOT)}")
    except Exception as e:
        print(f"❌ Failed: {file_path} - {e}")

def main():
    print(f"🚀 Starting SOTA Swarm Refactor in {ROOT}...")
    for ext in ["*.py", "*.sh", "*.ps1", "*.md", "heiwa", "*.plist", "railway.toml", "wrangler.toml", "*.yml", "Dockerfile"]:
        for file_path in ROOT.rglob(ext):
            if ".git" in str(file_path) or ".venv" in str(file_path):
                continue
            refactor_file(file_path)

if __name__ == "__main__":
    main()
