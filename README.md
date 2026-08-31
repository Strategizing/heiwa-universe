# Heiwa

[![CI](https://github.com/Heiwa-Limited/heiwa-universe/actions/workflows/ci.yml/badge.svg)](https://github.com/Heiwa-Limited/heiwa-universe/actions/workflows/ci.yml)
[![Docs](https://github.com/Heiwa-Limited/heiwa-universe/actions/workflows/pages.yml/badge.svg)](https://github.com/Heiwa-Limited/heiwa-universe/actions/workflows/pages.yml)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

Heiwa is a local-first AI operating layer. The installed `heiwa` runtime is the product center, Rust owns the execution path, and this repository is being hardened for GitHub-native distribution rather than hosted-platform theater.

## What Heiwa Does

> Heiwa watches what matters, summarizes what changed, stages what needs action, executes what is safe, and proves what happened.

Three planes compose one flow:

- **Intake** — operator command bar and passive feeds (mail, calendar, messages, files, runtime alerts).
- **Execution** — DREX routes work to local models, provider CLIs, tools, and connectors under leases and approval gates.
- **Evidence** — every read or action emits a source-linked receipt into the local JSONL journal; Lance provides derived recall.

Maturity is uneven across planes today; see [`HEIWA.md`](HEIWA.md#the-three-planes) for current vs target.

## One-Sentence Truth

`heiwa` is the installed product surface, DREX is the internal execution kernel, local JSONL is evidence truth, Lance is derived recall, and GitHub is source, CI, and distribution.

## Current Repo Focus

- Installed runtime: `apps/heiwa_shell/`
- Core execution and routing: `apps/heiwa_core/`, `crates/heiwa_loop/`, `crates/heiwa_session/`
- Provider normalization: `crates/heiwa_provider/`
- Terminal UX: `crates/heiwa_tui/`, `crates/heiwa_repl/`
- Evidence journal and recall: `crates/heiwa_evidence/`, `crates/heiwa_embed/`
- GitHub distribution surfaces: Actions, Pages, and release metadata

## Architecture

| Layer              | Canonical meaning                                                 | Location                                        |
| ------------------ | ----------------------------------------------------------------- | ----------------------------------------------- |
| **Heiwa**          | Company and product identity                                      | Repo root                                       |
| **`heiwa`**        | Primary installed runtime and operator surface                    | `apps/heiwa_shell/`                             |
| **DREX**           | Internal execution kernel and routing substrate                   | `apps/heiwa_core/`                              |
| **Local evidence** | Canonical JSONL journal plus derived Lance recall index           | `crates/heiwa_evidence/`, `crates/heiwa_embed/` |
| **Rust runtime**   | Volatile execution: provider supervision and candidate generation | `crates/`                                       |

> Rust proposes and executes, local text truth records, Lance recalls, `heiwa` presents.

## Install

```bash
curl -fsSL https://heiwa.ltd/install | sh
```

The installer verifies a SHA-256 checksum from the release's `checksums.txt`
before it moves anything into place, and refuses an archive containing links or
unsupported entry types. It installs under `~/.heiwa` — set `HEIWA_HOME` to an
absolute path to change that — and prints the exact binary, cockpit, and app
paths it wrote.

```bash
export PATH="$HOME/.heiwa/bin:$PATH"
heiwa doctor
heiwa app start --no-open
```

Pin a version with `HEIWA_VERSION=0.2.0`. To read the script before running it,
fetch it first: `curl -fsSL https://heiwa.ltd/install -o heiwa-install.sh`.

What each platform gets today:

| Platform | CLI | Desktop app | Source |
| --- | --- | --- | --- |
| macOS aarch64 | yes | yes, placed in `/Applications` (or `~/Applications`) | installer or [Releases](https://github.com/Heiwa-Limited/heiwa-universe/releases) |
| Linux x86_64 | yes | not yet | installer or Releases |
| Windows x86_64 | yes | not yet | [Releases](https://github.com/Heiwa-Limited/heiwa-universe/releases) archive |
| Container | yes | not applicable | `ghcr.io/heiwa-limited/heiwa` |

The macOS app ships as the updater's own signed tarball rather than a `.dmg`: a
browser download sets `com.apple.quarantine` and Gatekeeper blocks an unsigned
app, so a `.dmg` on the releases page would be a broken artifact behind the most
obvious download button. `curl` and the in-app updater set no quarantine bit.

Release archives carry build provenance attestations; the container image also
carries an SBOM. Every published release is installed end-to-end on Linux and
macOS by
[`public-install-smoke.yml`](.github/workflows/public-install-smoke.yml) before
the release workflow finishes.

Updating:

```bash
heiwa app update --dry-run --json   # show the plan
heiwa app update                    # apply it
```

## Build from Source

For contributors. Requires the Rust toolchain in
[`BUILD_MATRIX.md`](BUILD_MATRIX.md); this is not the install path.

```bash
# Verify the local toolchain baseline
bash scripts/check_runtime_baseline.sh

# Build the installed runtime
cargo build -p heiwa-shell

# Run install and doctor
cargo run -p heiwa-shell --bin heiwa -- install
cargo run -p heiwa-shell --bin heiwa -- doctor

# Inspect providers and auth state
cargo run -p heiwa-shell --bin heiwa -- providers
cargo run -p heiwa-shell --bin heiwa -- auth status
```

## Platform Lane

- PR CI pairs a sub-minute feedback lane with a bounded Linux Rust test/Clippy lane; protected-main certification compiles macOS/Windows and certifies desktop, Lance, and security before release.
- Docs publish through GitHub Pages on release tags.
- Cargo manifests now carry shared package metadata for release readiness.
- Release archives include the Apache-2.0 license and contributor materials.
- Contributor, security, pull request, and issue templates live under `.github/` and `SECURITY.md`.

## Read First

- [`HEIWA.md`](HEIWA.md)
- [`docs/product-contract.md`](docs/product-contract.md)
- [`docs/capability-fabric.md`](docs/capability-fabric.md)
- [`docs/local-self-operation.md`](docs/local-self-operation.md)
- [`AGENTS.md`](AGENTS.md)
- [`BUILD_MATRIX.md`](BUILD_MATRIX.md)
- [`SECURITY.md`](SECURITY.md)
- [`docs/`](docs/)
