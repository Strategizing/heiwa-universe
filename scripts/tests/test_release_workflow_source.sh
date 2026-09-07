#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
workdir="$(mktemp -d "${TMPDIR:-/tmp}/heiwa-release-source.XXXXXX")"
trap 'rm -rf "$workdir"' EXIT

# Exercise the actual workflow shell, with only an isolated Git history and
# version declarations. No network, release publication, or installer execution.
ruby -ryaml - "$repo_root/.github/workflows/release.yml" "$workdir" <<'RUBY'
workflow = YAML.safe_load(File.read(ARGV[0]), aliases: true)
steps = workflow.fetch("jobs").fetch("metadata").fetch("steps")
File.write("#{ARGV[1]}/resolve.sh", steps.find { |step| step["id"] == "meta" }.fetch("run"))
stage = steps.find { |step| step["id"] == "release-source" }
File.write("#{ARGV[1]}/stage.sh", stage ? stage.fetch("run") : ":\n")
%w[check_release_version_sync check_installer_version_pin].each do |script|
  step = steps.find { |entry| entry["run"] == "bash scripts/#{script}.sh \"$RELEASE_VERSION\"" }
  abort "release workflow must run #{script} against staged release data" unless step
end
RUBY

fixture="$workdir/repo"
mkdir -p "$fixture/scripts"
cd "$fixture"
git init -q -b main
git config user.name 'Release Source Fixture'
git config user.email 'release-fixture@example.invalid'
git config commit.gpgsign false
git config core.excludesFile /dev/null
git config core.hooksPath /dev/null

write_version() {
  local version="$1"
  for path in apps/heiwa_shell/Cargo.toml apps/heiwa_core/Cargo.toml \
    apps/heiwa_app/desktop/src-tauri/Cargo.toml; do
    mkdir -p "$(dirname "$path")"
    printf '[package]\nname = "fixture"\nversion = "%s"\n' "$version" >"$path"
  done
  for path in apps/heiwa_app/desktop/src-tauri/tauri.conf.json \
    apps/heiwa_app/desktop/package.json; do
    printf '{"version":"%s"}\n' "$version" >"$path"
  done
  mkdir -p apps/heiwa_app/clients/web
  printf 'pinned_version="%s"\n' "$version" >apps/heiwa_app/clients/web/install
  cp apps/heiwa_app/clients/web/install apps/heiwa_app/clients/web/install.sh
}

# Tag scripts must never execute. The validators used by publication come
# from the current trusted main checkout; only release metadata is restored.
for script in check_release_version_sync check_installer_version_pin; do
  printf 'touch "$RELEASE_SOURCE_EXECUTED"\nexit 0\n' >"scripts/$script.sh"
done
write_version 0.1.0
git add .
git commit -qm 'Version 0.1.0 source'
git -c tag.gpgsign=false tag -a v0.1.0 -m 'Matching ancestor tag'
git -c tag.gpgsign=false tag -a v0.2.0 -m 'Mismatched ancestor tag'

# A separate tag has matching artifact versions but a stale installer pin.
write_version 0.3.0
printf 'pinned_version="0.2.0"\n' >apps/heiwa_app/clients/web/install
cp apps/heiwa_app/clients/web/install apps/heiwa_app/clients/web/install.sh
git add .
git commit -qm 'Version 0.3.0 with stale installer'
git -c tag.gpgsign=false tag -a v0.3.0 -m 'Stale installer ancestor tag'

write_version 0.2.0
cp "$repo_root/scripts/check_release_version_sync.sh" scripts/
cp "$repo_root/scripts/check_installer_version_pin.sh" scripts/
git add .
git commit -qm 'Current main metadata and trusted validators'
main_commit="$(git rev-parse HEAD)"
export RELEASE_SOURCE_EXECUTED="$workdir/tag-script-executed"

check_tag() {
  local tag="$1"
  git restore --source="$main_commit" --worktree -- .
  export DISPATCH_TAG="$tag" GITHUB_OUTPUT="$workdir/output"
  : >"$GITHUB_OUTPUT"
  bash "$workdir/resolve.sh" || return
  export RELEASE_COMMIT RELEASE_VERSION
  RELEASE_COMMIT="$(sed -n 's/^commit=//p' "$GITHUB_OUTPUT")"
  RELEASE_VERSION="$(sed -n 's/^version=//p' "$GITHUB_OUTPUT")"
  bash "$workdir/stage.sh" || return
  bash scripts/check_release_version_sync.sh "$RELEASE_VERSION" &&
    bash scripts/check_installer_version_pin.sh "$RELEASE_VERSION"
}

if check_tag v0.2.0 >"$workdir/log" 2>&1; then
  echo 'release gates accepted mismatched tag bytes because main matched the release' >&2
  exit 1
fi
grep -Fq 'declares version 0.1.0, release is 0.2.0' "$workdir/log" || { cat "$workdir/log" >&2; exit 1; }

check_tag v0.1.0 >"$workdir/log" 2>&1
if check_tag v0.3.0 >"$workdir/log" 2>&1; then
  echo 'release gates accepted a stale installer in the tag' >&2
  exit 1
fi
grep -Fq 'public installer fallback pin is stale' "$workdir/log"
[[ ! -e "$RELEASE_SOURCE_EXECUTED" ]] || {
  echo 'release validation executed a script from the tag' >&2
  exit 1
}

# Pin every build checkout to the commit the metadata gate certified, and
# route the post-publish container through the verified-release packaging lane.
ruby -ryaml - "$repo_root/.github/workflows/release.yml" "$repo_root/.github/workflows/container.yml" <<'RUBY'
release, container = ARGV.map { |path| YAML.safe_load(File.read(path), aliases: true) }
%w[cockpit desktop-bundle build].each do |id|
  checkout = release.fetch("jobs").fetch(id).fetch("steps").find { |step| step["uses"].to_s.start_with?("actions/checkout@") }
  abort "#{id} must check out the resolved release commit" unless checkout.fetch("with")["ref"] == '${{ needs.metadata.outputs.commit }}'
end
call = release.fetch("jobs").fetch("container")
abort "release container must reuse verified release packaging" unless call["uses"] == "./.github/workflows/container.yml" && !call.key?("steps")
abort "container must wait for published release bytes" unless Array(call["needs"]).sort == %w[metadata publish]
abort "container must receive resolved tag" unless call.fetch("with")["tag"] == '${{ needs.metadata.outputs.tag }}'
abort "container caller must grant package publication" unless call.fetch("permissions")["packages"] == "write"
triggers = container["on"] || container[true]
abort "container must be callable with a required tag" unless triggers.fetch("workflow_call").fetch("inputs").fetch("tag") == {
  "description" => "Existing annotated release tag to containerize", "required" => true, "type" => "string"
}
steps = container.fetch("jobs").fetch("build").fetch("steps")
abort "container must stage verified release bytes" unless steps.any? { |step| step["run"].to_s.include?("bash packaging/scripts/stage_release_container.sh") }
build = steps.find { |step| step["uses"].to_s.start_with?("docker/build-push-action@") }.fetch("with")
abort "container must preserve verified packaging and attestations" unless build["file"] == "packaging/apps/heiwa_shell/Dockerfile.release" &&
  build["platforms"] == "linux/amd64" && build["provenance"] == "mode=max" && build["sbom"] == true
RUBY

echo 'Release workflow source and container reuse tests passed.'
