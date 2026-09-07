# Delivery Quality Implementation Plan

**Goal:** Make Heiwa development and publishing produce inspectable, bounded,
source-specific evidence and follow coherent authorization rules.

**Architecture:** Refactor existing entry points and workflows. Separate
command execution/evidence from the local check inventory. Share sidecar
verification between local and remote checks. Keep protected PR promotion.

**Tech stack:** Python standard library, shell, GitHub Actions YAML, existing
Ruby workflow validation. No runtime backend or model change.

**Spec:** `docs/superpowers/specs/2026-09-07-delivery-quality-design.md`.

## Execution

- [x] Add behavior regressions in `scripts/tests/test_ci_local.py` using real
  subprocesses and disposable Git repositories. Run with
  `python3 -m unittest discover -s scripts/tests -p test_ci_local.py`.
- [x] Replace `scripts/check_ci_local.sh` internals with a thin interpreter
  bootstrap; implement `Check`, `run_checks`, and the current check inventory
  in `scripts/check_ci_local.py`. Preserve mandatory gates, add strict docs,
  sidecar, instruction sync, and test-inventory validation. Write versioned
  receipts atomically under ignored private verification directories.
- [x] Add `scripts/check_python_sidecar.sh`; call it from local CI and a
  required dependency of the existing PR aggregate. Test positive and negative
  aggregate results; retain the protected status context name.
- [x] Migrate active publishing workflows to GitHub-hosted runners and extend
  the existing runner policy with regression fixtures for all workflow/matrix
  shapes. Keep dispatch-only vendor canaries separate from publication.
- [x] Reconcile AGENTS, provider instructions, canonical agent templates, and
  the baseline workflow. Regenerate derived agent files and run sync checks.
- [ ] Review all changes for duplicate mechanics, softened gates, misleading
  maturity claims, and secret exposure. Commit the cohesive implementation on
  the experimental branch, then run the full local gate at that revision.
- [ ] Publish a PR to `dev`, inspect current CI and review threads, repair any
  reproduced findings, and promote only after the configured gates pass.
  Carry the same evidence requirements through production publication.

Independent instruction and runner-policy repairs run through the parallel
agents workflow. The parent integrates them with verification changes and
owns remote publication. Existing worktrees and durable user state remain
available until replacement acceptance is established.
