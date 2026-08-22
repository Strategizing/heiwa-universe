# L5 Mesh Runtime — Task Ledger

Contract: `docs/superpowers/specs/2026-08-20-heiwa-mesh-runtime-design.md`
Started: 2026-08-22

Status is what is true at HEAD, not what is intended. A row moves to done only
when its verification runs.

## Why this before L4

The mesh spec says L3 remains the immediate work and L4 follows, and that it
"is the L5 specification, written now because L3 and L4 must not be built in a
shape the mesh cannot carry." Two of the four constraints it places on L4 —
`Owner::User { node_id }` and every domain event carrying `work_id` — need a
`node_id` that exists. L3 steps 6–7 are blocked on an external Google account
change that only Devon can make. So the unblocked, ordering-correct work is the
part of L5 that needs one machine.

## Scope of L5.0a

**Built:** node identity (keypair, fingerprint, versioned record), the live
`CapabilityAdvertisement` container with expiry, the signed and hash-chained
`MeshEnvelope`, a hybrid logical clock, a peer registry, and the surfaces that
read them.

**Not built, and not claimed anywhere in the tree:** peer transport, pairing,
advertisement exchange, anti-entropy replication, `Work`, `ProviderSession`
host affinity, `SurfaceInstance`/`ControlLease`, `AttentionItem`. D4 (peer
transport) is still unresolved and still needs a spike.

A node established by this work has **no peers and cannot reach another
device.** Both the CLI and the app say so in those words.

## Steps

| # | Step | Status | Verification |
|---|---|---|---|
| 1 | `heiwa_mesh` — node keypair, `NodeId` fingerprint, versioned record | **done** | 21 unit + 13 integration tests; `cargo test -p heiwa_mesh` |
| 2 | Signing key in the OS credential store, never under the config root | **done** | `the_signing_key_never_lands_under_the_configuration_root`; `KeyStore` is injected so CI proves it without a keychain |
| 3 | `MeshEnvelope` — signed, hash-chained, tamper-rejecting | **done** | tampered payload, rewritten privacy class, and foreign-key verification all refused; chain links asserted |
| 4 | `CapabilityAdvertisement` — expiry and republish semantics | **done** | stale advertisement is never fresh; republish is driven by content, not by the heartbeat clock |
| 5 | Hybrid logical clock | **done** | 5 tests including a backwards wall clock and a skewed peer |
| 6 | Peer registry, revocation-aware | **done** | a revoked peer is not active and stays verifiable |
| 7 | `heiwa mesh status` / `heiwa mesh enroll` | **done** | 6 tests; enrolment is idempotent and explicit, never at boot |
| 8 | Machine perspective reads mesh state instead of asserting it | **done** | `machine_perspective` tests; an unreadable registry reports `unknown`, not `local_only` |
| 9 | Desktop surface renders all three sync states | **done** | `does not claim peer sync when the mesh state could not be read`; 83/83 vitest |
| 10 | Peer pairing transport (D4) | blocked (needs a spike) | — |
| 11 | Advertisement exchange and anti-entropy | pending | needs step 10 |
| 12 | `Work` above `Task` | pending | — |

## Decisions

- **D3 resolved: sibling record.** The keypair lives in `mesh-node.json` beside
  `local-identity.json`, bound to the same `installation_id`, rather than
  inside `LocalIdentity`. L2 made the identity record contact-free and safe to
  read from anywhere; putting signing material in it would make every reader of
  a display name a reader of key metadata.

- **AD-31** `heiwa_mesh` never resolves a configuration root. Every entry point
  takes `&Path`. The shell resolves once through `crate::home::heiwa_runtime_dir()`
  and passes it down. A second resolver is what `scripts/check_l0_acceptance.sh`
  exists to catch, and a crate that resolves its own root cannot be tested
  against a temporary one.

- **AD-32** The signing key store is a trait, not `heiwa_vault` directly. Same
  reason as AD-20: a component that can only talk to the real OS keychain
  cannot be proven in CI, and the Linux runner has no Secret Service at all.
  `VaultKeyStore` is the shipped implementation; its one non-delegating
  decision — a missing entry is `Ok(None)`, a broken backend is `Err` — is a
  free function with its own tests. AD-25's failure mode, one layer down.

- **AD-33** The signature digest does not go through `serde_json::to_string`.
  `serde_json::Map` is a `BTreeMap` by default and an `IndexMap` when any crate
  in the build enables `preserve_order` — which `heiwa_mcp` already does
  transitively through `schemars`. Cargo unifies features across the graph, so
  a digest computed that way could change meaning because of an unrelated
  crate's dependency. Object keys are sorted by a local canonicaliser instead,
  and every field is length-prefixed so no two field sequences collide.

- **AD-34** Enrolment is an explicit command, never a boot step. The spec's
  MacBook-first bootstrap says in as many words that it "does not create a node
  keypair", and the security shape says enrolment is a user action. A key
  minted silently at first launch would put material in the user's keychain
  that they never asked for and that nothing can yet use.

- **AD-35** `causal_parents` is carried in the envelope and read by nothing.
  This is deliberate and is the one place this slice ships an inert field:
  causality cannot be backfilled into an already-replicated log, and the
  envelope's schema version is what a peer will negotiate against. It is named
  here so it is not mistaken for working anti-entropy.

- **AD-36** `AgentIdentity.node` was left unpopulated. The spec expects it to
  become a `node_id`, but the type has exactly one occurrence in the tree — its
  own definition — and is constructed nowhere. Populating a field no code
  writes would be a change with no behavior. It becomes real when the worker
  and task paths are node-attributed, which needs step 10.

## Notes from implementation

The desktop surface had a latent defect that only became visible once the
runtime could answer a third value. `sync_status === "local_only" ? "sync local
only" : "peer enrolled"` renders *every* non-`local_only` state as peer
enrolled — so a machine that could not read its own mesh state would have told
the user it was paired with another device. The test was written before the
backend could produce `unknown`, and it failed exactly there.

The general shape: a binary ternary over an open string enum is a lie waiting
for its third case. The fix is exhaustive mapping with an explicit fallback
that admits ignorance.

A second defect survived a green test suite and was caught only by running the
binary: `#[serde(rename_all = "snake_case")]` renders the variant `MacOS` as
`mac_o_s`, so `heiwa mesh status` printed a platform string that matches
nothing else in the tree — not `env::consts::OS`, not the machine manifest, not
the desktop surface. Every test asserted against the enum, so none of them
noticed. The variant now carries an explicit `rename`, with a test that pins
all five wire strings.

Both findings say the same thing about verification: tests written against the
types you control cannot catch a disagreement with the types you do not. Run
the command.
