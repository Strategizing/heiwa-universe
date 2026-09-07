#!/usr/bin/env bash
set -euo pipefail

root_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root_dir"

expect_file() {
  local path="$1"
  if [[ ! -f "$path" ]]; then
    echo "Missing required baseline file: $path" >&2
    exit 1
  fi
}

expect_file "rust-toolchain.toml"
expect_file ".nvmrc"
expect_file ".node-version"
expect_file "package.json"
expect_file ".env.example"
expect_file ".github/workflows/ci.yml"
expect_file ".github/workflows/deploy.yml"
expect_file "apps/heiwa_core/Dockerfile"
expect_file "apps/heiwa_shell/Dockerfile"

required_rust_channel="1.95.0"
required_node_version="26.0.0"

rust_channel="$(awk -F'"' '/channel = / { print $2 }' rust-toolchain.toml)"
if [[ -z "$rust_channel" ]]; then
  echo "Could not parse Rust channel from rust-toolchain.toml" >&2
  exit 1
fi

if [[ "$rust_channel" != "$required_rust_channel" ]]; then
  echo "rust-toolchain.toml must pin Rust $required_rust_channel, found $rust_channel" >&2
  exit 1
fi

rust_major_minor="${rust_channel%.*}"

for dockerfile in apps/heiwa_core/Dockerfile apps/heiwa_shell/Dockerfile; do
  builder_line="$(grep -E '^FROM rust:[0-9]+\.[0-9]+-slim AS rust-builder$' "$dockerfile" || true)"
  if [[ -z "$builder_line" ]]; then
    echo "Could not find rust-builder image pin in $dockerfile" >&2
    exit 1
  fi

  builder_version="${builder_line#FROM rust:}"
  builder_version="${builder_version%-slim AS rust-builder}"
  if [[ "$builder_version" != "$rust_major_minor" ]]; then
    echo "Rust baseline drift: rust-toolchain.toml=$rust_channel but $dockerfile builder pin=$builder_version" >&2
    exit 1
  fi
done

while IFS= read -r copy_line; do
  read -r -a copy_parts <<<"$copy_line"
  for ((index = 1; index < ${#copy_parts[@]} - 1; index++)); do
    copy_source="${copy_parts[$index]%/}"
    if [[ ! -e "$copy_source" ]]; then
      echo "apps/heiwa_shell/Dockerfile copies missing source: $copy_source" >&2
      exit 1
    fi
  done
done < <(grep -E '^COPY ' apps/heiwa_shell/Dockerfile | grep -Ev '^COPY --from=')

for container_requirement in \
  protobuf-compiler \
  libdbus-1-dev \
  '--features heiwa-shell/lance' \
  'COPY apps/heiwa_app/desktop/src-tauri/ ./apps/heiwa_app/desktop/src-tauri/'; do
  if ! grep -Fq -- "$container_requirement" apps/heiwa_shell/Dockerfile; then
    echo "apps/heiwa_shell/Dockerfile is missing full-runtime requirement: $container_requirement" >&2
    exit 1
  fi
done

if grep -Eq 'cargo build[^|]*&& strip[^|]*\|\| true' apps/heiwa_shell/Dockerfile; then
  echo "apps/heiwa_shell/Dockerfile must not let strip failure handling swallow cargo build failures" >&2
  exit 1
fi

nvmrc_version="$(tr -d '[:space:]' < .nvmrc)"
node_version_file="$(tr -d '[:space:]' < .node-version)"

if [[ "$nvmrc_version" != "$node_version_file" ]]; then
  echo ".nvmrc ($nvmrc_version) does not match .node-version ($node_version_file)" >&2
  exit 1
fi

if [[ "$nvmrc_version" != "$required_node_version" ]]; then
  echo ".nvmrc/.node-version must pin Node $required_node_version, found $nvmrc_version" >&2
  exit 1
fi

if ! grep -q '"typecheck"' package.json; then
  echo "package.json must define a root typecheck script" >&2
  exit 1
fi

if ! grep -q '"workspaces"' package.json; then
  echo "package.json must define npm workspaces" >&2
  exit 1
fi

npm_workspaces="$(
  node -e '
    const { globSync } = require("node:fs");
    const pkg = require("./package.json");
    const patterns = Array.isArray(pkg.workspaces)
      ? pkg.workspaces
      : pkg.workspaces?.packages ?? [];
    const workspaces = patterns.flatMap((pattern) =>
      globSync(pattern, { exclude: ["**/node_modules/**"] })
    );
    process.stdout.write([...new Set(workspaces)].join("\n"));
  '
)" || {
  echo "Could not parse npm workspaces from package.json" >&2
  exit 1
}

while IFS= read -r workspace; do
  [[ -z "$workspace" ]] && continue
  if [[ -f "$workspace/package-lock.json" ]]; then
    echo "npm workspace must use the root package-lock.json: $workspace/package-lock.json" >&2
    exit 1
  fi
done <<< "$npm_workspaces"

required_node_major="${required_node_version%%.*}.x"
if ! grep -q "\"node\": \"${required_node_major}\"" package.json; then
  echo "package.json engines must pin Node ${required_node_major}" >&2
  exit 1
fi

ruby -ryaml - <<'RUBY'
failures = []
Dir.glob(".github/workflows/*.{yml,yaml}").sort.each do |path|
  workflow = YAML.safe_load(File.read(path), aliases: true)
  workflow.fetch("jobs", {}).each do |job_id, job|
    Array(job["steps"]).each do |step|
      next unless step["uses"].to_s.start_with?("actions/setup-node@")

      settings = step.fetch("with", {})
      unless settings["node-version-file"] == ".nvmrc" && !settings.key?("node-version")
        failures << "#{path} #{job_id}: setup-node must use .nvmrc without a version override"
      end
    end
  end
end
abort failures.join("\n") unless failures.empty?
puts "Every workflow Node setup uses the repository baseline."
RUBY

if ! grep -Eq "actions/setup-node@[0-9a-f]{40}[[:space:]]+# v[0-9]+" .github/workflows/certification.yml; then
  echo ".github/workflows/certification.yml must set up Node with an immutable commit pin" >&2
  exit 1
fi

for workflow in .github/workflows/ci.yml .github/workflows/deploy.yml; do
  repo_hygiene_job="$(
    awk '
      /^  repo-hygiene:/ { in_job = 1 }
      in_job && /^  [[:alnum:]_-]+:/ && $1 != "repo-hygiene:" { exit }
      in_job { print }
    ' "$workflow"
  )"

  if ! grep -Eq "actions/setup-node@[0-9a-f]{40}[[:space:]]+# v[0-9]+" <<< "$repo_hygiene_job"; then
    echo "$workflow repo-hygiene must set up Node with an immutable commit pin" >&2
    exit 1
  fi

  if ! grep -Eq "node-version-file: ['\"]?\\.nvmrc['\"]?" <<< "$repo_hygiene_job"; then
    echo "$workflow repo-hygiene must source Node from .nvmrc" >&2
    exit 1
  fi
done

bash scripts/check_workflow_pins.sh

for workflow in .github/workflows/ci.yml .github/workflows/certification.yml .github/workflows/deploy.yml; do
  if ! grep -Eq "toolchain: ${required_rust_channel}" "$workflow"; then
    echo "$workflow must install Rust $required_rust_channel" >&2
    exit 1
  fi
done

for required_env in HEIWA_MACHINE_AUTH_TOKEN HEIWA_JWT_SIGNING_SECRET HEIWA_STATE_BACKEND; do
  if ! grep -q "^${required_env}=" .env.example; then
    echo ".env.example is missing canonical variable: $required_env" >&2
    exit 1
  fi
done

bash scripts/tests/test_audit_operator_machine.sh

echo "Runtime baseline pins and workflow wiring are present."
