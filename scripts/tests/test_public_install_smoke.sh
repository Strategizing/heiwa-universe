#!/usr/bin/env bash
set -euo pipefail
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
ruby - "$repo_root" <<'RUBY'
require 'yaml'
require 'tmpdir'
require 'fileutils'
require 'open3'
workflow = YAML.safe_load(File.read(File.join(ARGV[0], '.github/workflows/public-install-smoke.yml')), aliases: true)
step = workflow.fetch('jobs').fetch('install').fetch('steps').find { |entry| entry['name'] == 'Install from the public edge' }.fetch('run')
Dir.mktmpdir('heiwa-public-smoke-fixture') do |temporary|
  bin = File.join(temporary, 'bin')
  FileUtils.mkdir_p(bin)
  File.write(File.join(bin, 'curl'), <<~'SCRIPT')
    #!/usr/bin/env bash
    set -euo pipefail
    while (( $# )); do
      case "$1" in
        --output) output="$2"; shift 2 ;;
        --dump-header) headers="$2"; shift 2 ;;
        *) shift ;;
      esac
    done
    cp "$INSTALLER_FIXTURE" "$output"
    printf 'HTTP/1.1 200 OK\n' > "$headers"
    printf '200'
  SCRIPT
  File.chmod(0700, File.join(bin, 'curl'))
  installer = File.join(temporary, 'installer.sh')
  File.write(installer, <<~'SCRIPT')
    #!/bin/sh
    set -eu
    # A fresh installer may do work before it creates its destination.
    sleep 0.2
    mkdir -p "$HEIWA_HOME/bin" "$HEIWA_HOME/app/cockpit-current"
    cp "$BINARY_FIXTURE" "$HEIWA_HOME/bin/heiwa"
    chmod +x "$HEIWA_HOME/bin/heiwa"
    printf '<!doctype html>' > "$HEIWA_HOME/app/cockpit-current/index.html"
    printf 'Installed fixture\n'
  SCRIPT
  binary = File.join(temporary, 'heiwa.sh')
  File.write(binary, <<~'SCRIPT')
    #!/bin/sh
    set -eu
    test "${HEIWA_HOME:-}" = "$EXPECTED_ROOT" || {
      echo 'runtime escaped the isolated installation root' >&2
      exit 9
    }
    case "$*" in
      --version) printf 'heiwa %s\n' "${FIXTURE_VERSION:-$HEIWA_VERSION}" ;;
      'app update --dry-run --json')
        printf '{"source_mode":"github-release","current_version":"%s"}\n' "$HEIWA_VERSION" ;;
      *) exit 8 ;;
    esac
  SCRIPT
  script = File.join(temporary, 'workflow.sh')
  File.write(script, step)
  ambient = File.join(temporary, 'ambient-root')
  FileUtils.mkdir_p(ambient)
  File.write(File.join(ambient, 'sentinel'), 'preserve')
  %w[fresh wrong-version].each do |scenario|
    runtime = File.join(temporary, scenario)
    env = {
      'PATH' => "#{bin}:#{ENV.fetch('PATH')}", 'RUNNER_TEMP' => temporary,
      'RUNNER_OS' => 'Linux', 'RUNTIME_ROOT' => runtime, 'HEIWA_HOME' => ambient,
      'EXPECTED_ROOT' => runtime, 'HEIWA_VERSION' => '0.3.0',
      'INSTALL_URL' => 'https://example.invalid/install',
      'INSTALLER_FIXTURE' => installer, 'BINARY_FIXTURE' => binary,
      'FIXTURE_VERSION' => scenario == 'wrong-version' ? '0.0.0' : '0.3.0',
    }
    output, error, status = Open3.capture3(env, 'bash', script)
    if scenario == 'fresh'
      abort "fresh isolated installation failed: #{output}#{error}" unless status.success?
      abort 'installation output was not retained' unless File.read(File.join(runtime, 'install.log')).include?('Installed fixture')
    else
      abort 'smoke accepted a binary reporting the wrong version' if status.success?
    end
    abort 'smoke touched ambient runtime state' unless Dir.children(ambient) == ['sentinel'] && File.read(File.join(ambient, 'sentinel')) == 'preserve'
  end
end
puts 'Public install smoke handles a fresh root, isolates runtime state, and rejects version mismatch.'
RUBY
