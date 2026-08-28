# Work Continuity Program 0 — Task Ledger

Contract: `docs/superpowers/specs/2026-08-27-heiwa-work-continuity-triple-design.md`
Product companion: `docs/design/2026-08-27-heiwa-persistent-world-product-map.md`
Started: 2026-08-27

Status is what is true at HEAD, not what is intended. A row moves to done only
when its verification runs.

Program 0 is the claim-truth layer. It ships before Effect Receipts because the
continuity design's publication gates are all assertions, and an assertion that
nothing can falsify is a marketing sentence wearing a schema.

## Release P0-a — Executable claim registry

| # | Step | Status | Verification |
|---|---|---|---|
| 1 | `heiwa_claims` crate and manifest schema | done | `cargo test -p heiwa_claims` |
| 2 | Verifier allowlist; manifests cannot supply a command line | done | `cargo test -p heiwa_claims --test claim_state` |
| 3 | Scope digest over tracked blobs, generalizing `# acceptance-scope:` | done | `cargo test -p heiwa_claims` |
| 4 | Provider-neutral evidence records under `claims/evidence/` | done | `cargo test -p heiwa_claims --test claim_manifest` |
| 5 | Computed state ladder: planned/implemented/verified/degraded/retired | done | `cargo test -p heiwa_claims --test claim_state` |
| 6 | `heiwa-claims` CLI: list, check, show, verify, verifiers | done | `bash scripts/check_claims.sh` |
| 7 | Seed manifests for claims true at HEAD | done | `bash scripts/check_claims.sh` |
| 8 | Repository gate | done | `bash scripts/check_claims.sh` |

### Drift detection shipped in P0-a

The continuity design lists six drift classes the registry must catch. Two are
covered by the in-process verifiers now:

| Drift class | Covered | Mechanism |
|---|---|---|
| Named symbols or files that no longer exist | yes | `symbols-present` |
| Canonical docs conflicting with architecture truth | yes | `text-absent` |
| Schemas without serialization and migration tests | no | needs P0-b receipt schemas |
| Public claims whose guest adapter is absent | no | needs Program 3 |
| Stale external-spec or provider assumptions | partial | `expiry.max_age_days` only |
| Evidence tied to an older incompatible source state | yes | scope digest + ancestry + verifier version |

Reported honestly rather than closed early: three of six is what P0-a proves.

## Release P0-b — Receipt taxonomy

Splits the current receipt noun before either half can be published. Four
distinct types exist today and none of them means "an external effect happened":

- `heiwa_receipts::Receipt` — cost-bearing model/tool call accounting
- `heiwa_protocol::RunReceipt`, `heiwa_protocol::ToolCallReceipt`
- `heiwa_evidence::PersistedRunReceipt`

| # | Step | Status | Verification |
|---|---|---|---|
| 1 | `CallReceipt` naming with a compatibility alias | pending | `cargo test -p heiwa_receipts` |
| 2 | Retire STDB mirror vocabulary from the receipt boundary | pending | `heiwa-claims verify heiwa.receipts.no-stdb-vocabulary` |
| 3 | `EffectReceiptV1` design fixtures, no effect claims yet | pending | `cargo test -p heiwa_evidence` |
| 4 | Registry claim distinguishing call accounting from effect proof | pending | `bash scripts/check_claims.sh` |

## Decisions

- **The registry is a consolidation, not an invention.** `# acceptance-scope:`
  plus `.claude/l*-accept-sha` plus `scripts/hooks/stop_ledger_gate.sh` already
  implement claim → verifier → source-bound evidence → scope-aware staleness.
  P0-a keeps that mechanic and drops its three limits: provider-specific stamp
  paths, one hard-coded gate per roadmap layer, and bash-only readability.

- **Verifiers live in Rust, not in manifests.** A manifest is data someone may
  propose; data must not execute. `cargo-test`'s package parameter is validated
  against the workspace members Cargo actually declares, so the one
  caller-supplied string cannot be anything Cargo would not already build.

- **Evidence certifies a commit, so `verify` refuses a dirty scope.** Same rule
  the acceptance scripts follow. Reading the working tree while naming a commit
  would produce evidence about a tree only the operator can see.

- **Both gates run for now.** The roadmap acceptance scripts are not deleted.
  A claim mechanism that has never disagreed with the one it replaces has not
  been tested.

- **The registry seeds only claims true at HEAD.** The receipt split is a
  Program 0-b claim and is absent from `claims/` until it exists. Seeding a
  failing claim to express intent would make the gate a thing people mute.
