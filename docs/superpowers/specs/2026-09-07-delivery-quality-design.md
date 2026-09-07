# Heiwa delivery quality refactor

Plane: Execution / Evidence. Authorized by the request to refactor how Heiwa is
developed and published. This design changes the development harness and
publishing pipeline; it does not claim unfinished runtime capabilities exist.

## Audit

- Current remote `dev` is `d9b18f46`; `main` is `86368da9`. Both are protected
  with required checks and enforced administrator rules. The prior verified
  provider/security repair ends at `896bc94e` on an experimental branch.
- Local CI overwrites one global `/tmp/heiwa_ci_step.log`, omits sidecar tests
  and strict docs build, and silently skips any missing acceptance script.
- Required PR CI does not execute the maintained Python sidecar tests.
- Release and certification workflows still reference the runner vendor that
  stopped accepting required Heiwa jobs. The runner validator only covers CI.
- Repo agent instructions contain conflicting approval rules, outdated provider
  claims, and verification rules that confuse local proof with publication.

## Chosen approach

Refactor the existing delivery path in place. A wholesale product rewrite would
discard tested runtime behavior without evidence that a replacement works.
Only changing instruction prose would leave the executable gaps intact. This
refactor instead combines corrected instructions with enforceable checks.

The local shell entry point remains stable. A standard-library Python runner
owns check execution and evidence: separate private directory per invocation,
one log per check, atomic JSON receipts, source identity at both ends, bounded
execution, and child-process cleanup. Shell remains bootstrap glue; Rust keeps
runtime authority. The receipt proves local checks only, never remote readiness.

L0-L2 are required. A missing required script fails. Work Fabric A1 is explicitly
deferred while its gate is absent. A successful command list cannot certify a
dirty checkout or a revision changed during verification. Interrupted checks
must retain partial evidence and cannot produce a passing receipt.

The default profile records native desktop compilation as deferred. `--full`
adds native desktop tests, strict Clippy, and a release build alongside Lance.
An explicit host target avoids the `Heiwa`/`heiwa` output collision on macOS.
Required remote desktop checks run regardless of local profile.

Sidecar checks use one shared locked command path locally and in PR CI. Their
result participates in the existing required aggregate without renaming that
status context. Runner policy covers every active workflow and static matrix
target. Release jobs preserve architecture, provenance, signatures, and
certification requirements.

## Instruction contract

User authorization persists across the normal steps needed to fulfill the
assigned outcome. Skill advice does not override it. Architecture follows
current code and explicit user intent; existing facts constrain claims, not the
right to replace a design. Verification scales with risk and still includes
all mandatory promotion checks. Durable user state and installed runtime
changes retain their separate authorization and active-work protections.

This applies the instruction audit, follow-through, and verification guidance
in [OpenAI's current model guide](https://developers.openai.com/api/docs/guides/latest-model),
retrieved 2026-09-07. It does not imply API migration or model entitlement.

## Acceptance

Behavior tests cover failed/missing checks, source changes, dirty trees,
deferred checks, isolated concurrent receipts, timeouts, and interruption.
Sidecar behavior tests and lint pass from its frozen dependency graph.
Runner-policy mutation tests reject unsupported labels, including matrices.
The full local gate passes on a clean committed revision. Publication uses
experimental-to-dev and dev-to-main PRs with current checks and review evidence;
GitHub remains the sole binary release authority.
