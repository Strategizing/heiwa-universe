#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

fail=0

require_file() {
  local file="$1"
  if [[ ! -f "$file" ]]; then
    echo "missing required file: $file" >&2
    fail=1
  fi
}

require_match() {
  local file="$1"
  local pattern="$2"
  local label="$3"

  require_file "$file"
  if [[ -f "$file" ]] && ! grep -Eq -- "$pattern" "$file"; then
    echo "release metadata check failed for $file: $label" >&2
    fail=1
  fi
}

require_no_match() {
  local file="$1"
  local pattern="$2"
  local label="$3"

  require_file "$file"
  if [[ -f "$file" ]] && grep -Eq -- "$pattern" "$file"; then
    echo "release metadata check failed for $file: $label" >&2
    fail=1
  fi
}

require_block_match() {
  local file="$1"
  local start_pattern="$2"
  local end_pattern="$3"
  local required_pattern="$4"
  local label="$5"
  local block

  require_file "$file"
  if [[ ! -f "$file" ]]; then
    return
  fi

  block="$(awk -v start="$start_pattern" -v end="$end_pattern" '
    $0 ~ start { in_block = 1 }
    in_block { print }
    in_block && $0 ~ end { exit }
  ' "$file")"

  if [[ -z "$block" ]] || ! grep -Eq -- "$required_pattern" <<<"$block"; then
    echo "release metadata check failed for $file: $label" >&2
    fail=1
  fi
}

require_match "LICENSE" "^ *Apache License$" "root Apache-2.0 license text is required"
require_match "Cargo.toml" 'license = "Apache-2\.0"' "workspace package license must be Apache-2.0"
require_match ".github/workflows/release.yml" 'cp README\.md CONTRIBUTING\.md CODE_OF_CONDUCT\.md LICENSE' "release archives must include LICENSE"
require_block_match ".github/workflows/release.yml" \
  'Releases are dispatched from main' \
  'fetch-tags: true' \
  '^[[:space:]]*ref: main$' \
  "release metadata validation must check out protected main"
require_match ".github/workflows/release.yml" 'git merge-base --is-ancestor "\$commit" HEAD' "release tags must resolve to commits on main"
require_match ".github/workflows/release.yml" 'actions/workflows/\$\{workflow_file\}/runs\?event=push&branch=main&head_sha=\$\{RELEASE_COMMIT\}' "release publication must query required workflows at the tagged commit"
require_match ".github/workflows/release.yml" 'required_workflows=\(ci\.yml certification\.yml\)' "release publication must require fast CI and full certification"
require_match ".github/workflows/release.yml" 'if \[\[ "\$conclusion" != "success" \]\]' "release publication must reject failed main certification"
require_block_match ".github/workflows/pages.yml" \
  '^  build:' '^  deploy:' \
  'uv run --locked --extra docs python -m mkdocs build --strict' \
  "published docs must use the verified lockfile and strict build"
require_block_match ".github/workflows/ci.yml" \
  '^  desktop-app:' '^  rust-static:' \
  'node-version-file: \.nvmrc' \
  "desktop CI must use the repository Node baseline"
require_file ".github/workflows/certification.yml"
require_match ".github/workflows/certification.yml" '^name: Heiwa Certification$' "heavy release proofs must use the certification workflow"
require_no_match ".github/workflows/ci.yml" 'name: (Lance Backend Certification|Desktop Shell Certification|Cross-Platform Rust Compilation|Multi-Ecosystem Security Certification)' "heavy release proofs must stay out of sub-minute CI"
require_match ".github/workflows/certification.yml" 'mozilla-actions/sccache-action@fc920bf0ec8de6ee65d409111f7ec508035751ba' "protected main must seed the on-demand Rust compiler cache"
require_match ".github/workflows/ci.yml" 'name: Rust Tests' "PR CI must run the Rust test suite"
require_match ".github/workflows/ci.yml" 'name: Rust Static Checks' "PR CI must run Rust static checks"
# `main` branch protection pins the `Rust Source Policy` status context by name.
# Renaming or deleting the job that reports it leaves the required check pending
# forever and no PR can merge, so the aggregate job is release metadata.
require_match ".github/workflows/ci.yml" 'name: Rust Source Policy' "main branch protection requires the Rust Source Policy status context"
require_block_match ".github/workflows/ci.yml" \
  '^  rust-source-policy:' \
  '^    runs-on:' \
  'needs: \[rust-tests, rust-static, desktop-app, python-tests\]' \
  "the Rust Source Policy context must aggregate every runtime promotion lane"
require_block_match ".github/workflows/ci.yml" \
  '^  rust-source-policy:' \
  '^    runs-on:' \
  '^ *if: always\(\)$' \
  "the Rust Source Policy context must report even when a lane fails or is skipped"
# Declaring a lane in `needs` only makes the aggregate wait for it. Without a
# result comparison the required context still passes while that lane fails,
# which is the exact hole this pins shut.
require_block_match ".github/workflows/ci.yml" \
  '^  rust-source-policy:' \
  '^  lint:' \
  '"\$DESKTOP_APP_RESULT" != "success"' \
  "the Rust Source Policy context must fail when the Desktop App lane fails"
require_block_match ".github/workflows/ci.yml" \
  '^  rust-source-policy:' '^  lint:' \
  '"\$PYTHON_TESTS_RESULT" != "success"' \
  "the required aggregate must fail when Python Tests fail or are skipped"
require_block_match ".github/workflows/ci.yml" \
  '^  python-tests:' '^  lint:' \
  'bash scripts/check_python_sidecar\.sh' \
  "PR CI must use the shared frozen sidecar checks"
# Scope each command to the job that must own it. A whole-file `require_match`
# would still pass if a command drifted from one Rust lane into the other, which
# would silently change what each required context actually proves.
require_block_match ".github/workflows/ci.yml" \
  '^  rust-tests:' '^  rust-static:' \
  'bash scripts/ci_rust_test_group\.sh --check' \
  "the Rust Tests lane must validate the Rust test inventory"
require_block_match ".github/workflows/ci.yml" \
  '^  rust-tests:' '^  rust-static:' \
  'cargo test --workspace --exclude heiwa-desktop --locked --no-default-features' \
  "the Rust Tests lane must execute the workspace test suite before merge"
require_block_match ".github/workflows/ci.yml" \
  '^  rust-static:' '^  rust-source-policy:' \
  'cargo fmt --all -- --check' \
  "the Rust Static Checks lane must check Rust formatting"
require_block_match ".github/workflows/ci.yml" \
  '^  rust-static:' '^  rust-source-policy:' \
  'cargo clippy --workspace --exclude heiwa-desktop' \
  "the Rust Static Checks lane must execute clippy before merge"
require_block_match ".github/workflows/ci.yml" \
  '^  rust-static:' '^  rust-source-policy:' \
  'run: cargo machete' \
  "the Rust Static Checks lane must check unused Rust dependencies before merge"
require_match ".github/workflows/certification.yml" 'name: Run Linux Rust tests' "protected main must execute the Rust test suite"
require_match ".github/workflows/certification.yml" 'name: Compile non-Linux Rust test targets' "protected main must compile macOS and Windows Rust tests"
require_match ".github/workflows/certification.yml" 'name: Rust Static Certification' "protected main must run Rust static certification"
require_match ".github/workflows/certification.yml" 'cargo clippy --workspace --exclude heiwa-desktop' "protected main must run clippy"
require_match ".github/workflows/certification.yml" 'run: cargo machete' "protected main must check unused Rust dependencies"
if ! ruby scripts/check_ci_job_deadlines.rb; then
  fail=1
fi
require_match ".github/workflows/release.yml" 'shared-key: \$\{\{ matrix\.target \}\}' "release caches must be stable per target"
require_match ".github/workflows/release.yml" '--features heiwa-shell/lance' "release binaries must explicitly include the Lance recall backend"
require_match ".github/workflows/release.yml" 'actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a' "release uploads must use the Node 24 artifact action"
require_match ".github/workflows/release.yml" 'actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c' "release downloads must use the Node 24 artifact action"
require_no_match ".github/workflows/release.yml" '^[[:space:]]+tags:$' "release publication must be explicitly dispatched from main so caches remain reusable"
require_match ".github/workflows/container.yml" 'git merge-base --is-ancestor "\$commit" HEAD' "container tags must resolve to commits on main"
require_file "apps/heiwa_shell/Dockerfile.release"
require_file "scripts/stage_release_container.sh"
require_match ".github/workflows/container.yml" 'bash packaging/scripts/stage_release_container\.sh' "containers must stage verified release bytes"
require_match ".github/workflows/container.yml" '^[[:space:]]*file: packaging/apps/heiwa_shell/Dockerfile\.release$' "container publication must use the trusted runtime-only packaging recipe"
require_no_match "apps/heiwa_shell/Dockerfile.release" 'cargo build|rust-builder|^FROM rust:' "release containers must not recompile Rust"
require_match ".github/workflows/container.yml" '^[[:space:]]*platforms: linux/amd64$' "container publication must match the currently certified architecture"
# The public installer resolves the newest release at run time and falls back to
# a literal pin. Nothing used to keep that pin fresh, so a release could ship
# while `curl https://heiwa.ltd/install | sh` still installed the previous
# version. The release gate now refuses a stale pin.
require_file "scripts/check_installer_version_pin.sh"
require_match ".github/workflows/release.yml" 'bash scripts/check_installer_version_pin\.sh "\$RELEASE_VERSION"' "releases must refuse a stale public installer fallback pin in the checkout"
require_match ".github/workflows/release.yml" 'bash scripts/check_installer_version_pin\.sh --served "\$RELEASE_VERSION"' "releases must refuse a stale installer on the public edge, which deploys separately"
require_match "apps/heiwa_app/clients/web/install" '^pinned_version="[0-9]+\.[0-9]+\.[0-9]+"$' "the public installer must keep a semantic fallback pin"
require_match "apps/heiwa_app/clients/web/install" 'resolve_latest_version' "the public installer must resolve the newest release at run time"
require_file "scripts/configure_public_installer_edge.sh"
require_file "scripts/check_public_installer_edge.sh"
require_match ".github/workflows/deploy.yml" 'bash scripts/check_public_installer_edge\.sh' "Cloudflare deploys must verify non-browser installer access"
require_match ".github/workflows/deploy.yml" 'echo "deploy_core=false"' "manual web deploys must not run the Rust release preflight"
require_match "scripts/configure_public_installer_edge.sh" '^zone_name=.*heiwa\.ltd' "installer edge exception must stay scoped to heiwa.ltd"
require_match "scripts/configure_public_installer_edge.sh" 'uri\.path in \{\\"/install\\" \\"/install\.sh\\"\}' "installer edge exception must stay scoped to the two public installer paths"
require_match "scripts/check_public_installer_edge.sh" '^installer_url=.*https://heiwa\.ltd/install' "installer edge check must target the public installer"
bash scripts/tests/test_check_public_installer_edge.sh
bash scripts/tests/test_public_install_smoke.sh
require_block_match ".github/workflows/release.yml" \
  'ARCHIVE_EXT.*zip.*then' \
  '^[[:space:]]*else$' \
  '^[[:space:]]*cd dist$' \
  "Windows archives must be created from inside dist so they have one release root"
require_block_match "scripts/package_release_sandbox.sh" \
  '^[[:space:]]*zip\)$' \
  '^[[:space:]]*;;$' \
  '^[[:space:]]*cd "\$dist_dir"$' \
  "sandbox Windows archives must have the same one-root layout as CI"

python_projects=(
  "pyproject.toml"
  "packages/heiwa_cli/pyproject.toml"
  "packages/heiwa_sdk/pyproject.toml"
  "apps/heiwa_trading/pyproject.toml"
  "runtime/python/pyproject.toml"
)

for project in "${python_projects[@]}"; do
  require_match "$project" 'license = \{ text = "Apache-2\.0" \}' "Python project license must be Apache-2.0"
done

node_projects=(
  "package.json"
  "apps/heiwa_app/package.json"
  "apps/heiwa_app/clients/cockpit/package.json"
)

for project in "${node_projects[@]}"; do
  require_match "$project" '"license": "Apache-2\.0"' "Node package license must be Apache-2.0"
done

if git ls-files 'Cargo.toml' '*Cargo.toml' '*pyproject.toml' '*package.json' \
  | xargs grep -n 'UNLICENSED' >/tmp/heiwa-release-metadata-unlicensed.$$ 2>/dev/null; then
  echo "UNLICENSED metadata remains in tracked releasable manifests:" >&2
  cat /tmp/heiwa-release-metadata-unlicensed.$$ >&2
  fail=1
fi
rm -f /tmp/heiwa-release-metadata-unlicensed.$$

if (( fail != 0 )); then
  exit 1
fi

echo "Release metadata check passed."
