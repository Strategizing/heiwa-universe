# Product Surface

> Canonical map of tracked repo paths to surface classes. This file is read by `scripts/audit_product_surface.sh`. Update it when a path changes class; do not move class boundaries without checking `HEIWA.md`.

**Last updated:** 2026-08-27
**Authority:** `HEIWA.md` defines what is product. This file labels tracked paths for repo hygiene and LOC accounting.

## Classes

| Class              | Meaning                                                                                             |
| ------------------ | --------------------------------------------------------------------------------------------------- |
| `product`          | Active surfaces shipping in `heiwa`, companion runtime UX, repo release, or maintained sub-products |
| `generated`        | Code or lockfiles emitted from a registered generator, package manager, or schema source            |
| `legacy`           | Old surfaces kept for migration/reference; not part of the public product contract                  |
| `reference`        | Plans, design docs, audits, ADRs, continuity notes, or historical context                           |
| `archive`          | Frozen snapshots or pointers to work no longer active in this repo                                  |
| `vendored`         | Third-party code copied into the repo                                                               |
| `runtime-artifact` | Logs, caches, spools, tmp data, local run output; should trend to zero tracked LOC                  |

## Path To Class

Longest prefix wins. Put narrower paths above broader parents when a child has a different class.

| Path                       | Class            |
| -------------------------- | ---------------- |
| `archive`                  | archive          |
| `vendor`                   | vendored         |
| `apps/heiwa_shell`         | product          |
| `apps/heiwa_core`          | product          |
| `apps/heiwa_app`           | product          |
| `apps/heiwa_orchestrator`  | product          |
| `apps/heiwa_trading`       | product          |
| `archive/apps/heiwa_dj`    | archive          |
| `apps/__init__.py`         | legacy           |
| `claims/evidence`          | generated        |
| `claims`                   | product          |
| `crates`                   | product          |
| `packages/heiwa_skills`    | product          |
| `packages/heiwa_sdk`       | product          |
| `packages/heiwa_protocol`  | product          |
| `packages/heiwa_cli`       | product          |
| `packages/heiwa_identity`  | product          |
| `packages/heiwa_knowledge` | legacy           |
| `packages/__init__.py`     | product          |
| `runtime/python`           | product          |
| `runtime/fleets`           | runtime-artifact |
| `runtime/spool`            | runtime-artifact |
| `runtime/logs`             | runtime-artifact |
| `connectors`               | product          |
| `.superpowers`             | reference        |
| `docs/superpowers`         | reference        |
| `docs/design`              | reference        |
| `docs/audit`               | reference        |
| `docs/standards`           | product          |
| `docs`                     | product          |
| `oss-lifts`                | reference        |
| `prototypes`               | reference        |
| `ops/research`             | reference        |
| `ops/docs_and_deps`        | vendored         |
| `ops`                      | product          |
| `scripts`                  | product          |
| `tests`                    | product          |
| `infra`                    | product          |
| `config`                   | product          |
| `bin`                      | product          |
| `policies`                 | product          |
| `memory`                   | reference        |
| `plans`                    | reference        |
| `.claude/agents`           | generated        |
| `.claude`                  | product          |
| `.codex`                   | product          |
| `.greptile`                | product          |
| `.grok`                    | product          |
| `.gemini/agents`           | generated        |
| `.gemini`                  | product          |
| `.github`                  | product          |
| `.ollama`                  | product          |
| `.openclaw`                | legacy           |
| `.wrangler`                | runtime-artifact |
| `.gitleaksignore`          | product          |
| `Cargo.lock`               | generated        |
| `Cargo.toml`               | product          |
| `package-lock.json`        | generated        |
| `package.json`             | product          |
| `uv.lock`                  | generated        |
| `pyproject.toml`           | product          |
| `requirements.txt`         | product          |
| `README.md`                | product          |
| `LICENSE`                  | product          |
| `HEIWA.md`                 | product          |
| `HEIWA_LTD_BLUEPRINT.md`   | reference        |
| `AGENTS.md`                | product          |
| `CLAUDE.md`                | product          |
| `GEMINI.md`                | product          |
| `IDENTITY.md`              | reference        |
| `SOUL.md`                  | reference        |
| `SECURITY.md`              | product          |
| `CONTRIBUTING.md`          | product          |
| `CONTRIBUTORS.md`          | product          |
| `CODE_OF_CONDUCT.md`       | product          |
| `BUILD_MATRIX.md`          | reference        |
| `PRODUCT_SURFACE.md`       | product          |
| `mkdocs.yml`               | product          |
| `biome.json`               | product          |
| `deno.json`                | product          |
| `tsconfig.base.json`       | product          |
| `rust-toolchain.toml`      | product          |
| `conftest.py`              | product          |
| `justfile`                 | product          |
| `.dockerignore`            | product          |
| `.env.example`             | product          |
| `.geminiignore`            | product          |
| `.gitignore`               | product          |
| `.mcp.json`                | product          |
| `.node-version`            | product          |
| `.nvmrc`                   | product          |
| `.pyre_configuration`      | product          |

## Notes

- `apps/heiwa_trading` is an active sub-product, not slop.
- `legacy/` was removed from the tracked tree on 2026-07-06. Legacy surfaces (heiwa_hub, heiwa_cli, heiwa_limbs, legacy packages) live in git history and in the local operator archive (`~/heiwa_archive/heiwa-universe-pruned-2026-07-06/`).
- `runtime/python` is source and remains product for now. Runtime spools, logs, and fleet start artifacts are `runtime-artifact`.
- `docs/audit` is reference but contains operational baselines. Do not delete entries without replacing their evidence.
- `vendor/` is reserved for intentionally tracked third-party code. Current untracked `vendor/oss-lifts` research material is quarantine and must not be added or removed without an explicit vendor-policy assignment.
- `packages/heiwa_skills` is product because it installs provider/runtime behavior used by current Heiwa operator surfaces.
- Generated bindings and lockfiles are not slop by default, but their LOC should stay reproducible from a source schema or package manifest.

## Audit Rule

The audit script walks `git ls-files`, finds the longest-prefix match in this table, sums LOC per class, and reports any unmatched paths as `unclassified`. The target for `unclassified` is zero.
