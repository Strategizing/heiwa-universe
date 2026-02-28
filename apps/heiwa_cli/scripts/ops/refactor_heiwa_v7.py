import os
from pathlib import Path

ROOT = Path(__file__).resolve().parents[4]

REPLACEMENTS = {
    "apps/heiwa_hub": "apps/heiwa_hub",
    "apps/heiwa_cli": "apps/heiwa_cli",
    "packages/heiwa_sdk": "packages/heiwa_sdk",
    "packages/heiwa_protocol": "packages/heiwa_protocol",
    "apps.heiwa_hub": "apps.heiwa_hub",
    "from heiwa_hub": "from heiwa_hub",
    "from heiwa_sdk": "from heiwa_sdk",
    "from heiwa_protocol": "from heiwa_protocol",
    "heiwa_hub": "heiwa_hub",
    "heiwa_cli": "heiwa_cli",
    "heiwa_web": "heiwa_web",
}

def refactor_file(file_path):
    try:
        content = file_path.read_text(errors="ignore")
        new_content = content
        for old, new in REPLACEMENTS.items():
            new_content = new_content.replace(old, new)
        
        # Absolute Path Fixes for SDK/Protocol
        if 'packages/heiwa_sdk' in new_content:
            pass # good
        elif 'packages/heiwa_sdk' in new_content:
            new_content = new_content.replace('packages/heiwa_sdk', 'packages/heiwa_sdk')
        
        if 'packages/heiwa_protocol' in new_content:
            pass # good
        elif 'packages/heiwa_protocol' in new_content:
            new_content = new_content.replace('packages/heiwa_protocol', 'packages/heiwa_protocol')

        if new_content != content:
            file_path.write_text(new_content)
            print(f"✅ Refactored: {file_path.relative_to(ROOT)}")
    except Exception as e:
        print(f"❌ Failed: {file_path} - {e}")

def main():
    print(f"🚀 Starting Definitive Swarm Refactor v7 in {ROOT}...")
    for ext in ["*.py", "*.sh", "*.ps1", "*.md", "heiwa", "*.plist", "railway.toml", "wrangler.toml", "*.yml", "Dockerfile"]:
        for file_path in ROOT.rglob(ext):
            if ".git" in str(file_path) or ".venv" in str(file_path):
                continue
            refactor_file(file_path)

if __name__ == "__main__":
    main()
