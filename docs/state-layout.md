# Heiwa State Layout

Canonical layout for the local-first state root used by the `heiwa` runtime.

Default location: `~/.heiwa/` (override via `HEIWA_HOME`).

## Top-Level

| Path                     | Owner    | Purpose                                                         |
| ------------------------ | -------- | --------------------------------------------------------------- |
| `~/.heiwa/config.toml`   | operator | Local profile, route prefs, BYOX registration defaults          |
| `~/.heiwa/accounts.json` | runtime  | Connected provider accounts (status, models, expiry refs)       |
| `~/.heiwa/local-identity.json` | runtime | Local per-installation identity established during first run |
| `~/.heiwa/identity.json` | runtime  | Optional Heiwa service-login identity and token; distinct from first-run identity |
| `~/.heiwa/mesh-node.json` | `heiwa mesh enroll` | This machine's mesh node record: fingerprint and public key only. The private key is in the OS credential store under service `heiwa-mesh`, never here |
| `~/.heiwa/mesh-peers.json` | peer enrolment (not built) | Enrolled peer nodes and revocations. Absent on every installation today; absence reads as no peers, an unreadable file reads as unknown |
| `~/.heiwa/machine.json`  | runtime  | Stable device id plus refreshed platform, hardware, runtime, and local capability manifest |
| `~/.heiwa/evidence/`     | runtime  | Canonical versioned JSONL evidence journals                     |
| `~/.heiwa/secrets/`      | runtime  | OS-keychain-backed secret refs; never raw secrets in plain JSON |
| `~/.heiwa/state/`        | runtime  | Mutable runtime state (see below)                               |
| `~/.heiwa/sessions/`     | runtime  | Session transcripts and per-session metadata                    |
| `~/.heiwa/logs/`         | runtime  | Rotating runtime logs                                           |
| `~/.heiwa/cache/`        | runtime  | Provider response caches, model lists, expensive lookups        |
| `~/.heiwa/bin/`          | install  | Helper binaries (`heiwa-route`, etc.)                           |
| `~/.heiwa/app/Heiwa.app` | install  | HOME-local primary user input/display launcher for Heiwa.app    |
| `~/.heiwa/state.db`      | runtime  | Optional SQLite ledger (quotas, evidence)                       |
| `~/.heiwa/state/lance/`  | runtime  | Derived local recall index; safe to rebuild from text truth     |

## `~/.heiwa/state/` Subtree

This subtree is the only place runtime mutation happens for life/workers/approvals/evidence.

| Path                                  | Writer                           | Reader                                             |
| ------------------------------------- | -------------------------------- | -------------------------------------------------- |
| `state/workers.json`                  | `heiwa workers heartbeat`        | `heiwa workers status`, `heiwa app runtime status` |
| `state/dispatch/requests/`            | runtime, brokers                 | `heiwa approvals list`, `heiwa approvals show`     |
| `state/dispatch/approvals/decisions/` | `heiwa approvals decide`         | runtime brokers, audit                             |
| `state/dispatch/results/`             | runtime                          | audit                                              |
| `state/connectors/apple_calendar.json` | explicit connector enrollment   | Calendar resource/read/write gates                 |
| `state/calendar/`                     | calendar runtime                | Calendar UI, approval executor, connector audit    |
| `state/evidence/<utc-date>/`          | runtime, brokers                 | audit                                              |
| `state/health/doctor_latest.json`     | `heiwa doctor`                   | UI, CI                                             |
| `state/inventory/`                    | runtime                          | `heiwa providers`, `heiwa models`                  |
| `state/schedulers/`                   | scheduler                        | audit                                              |
| `state/life/readmodel.json`           | `heiwa life import` (when wired) | `heiwa life today`, `heiwa life status`            |
| `state/mail/headers.jsonl`            | mail bridge (planned)            | `heiwa life today`, urgency triage                 |
| `state/locks/`                        | runtime                          | runtime                                            |
| `state/net/`                          | runtime                          | telemetry                                          |
| `state/resources/`                    | runtime                          | scheduler                                          |

## Hard Rules

1. **No raw provider secrets in `state/`**. Use `~/.heiwa/secrets/` keychain refs only.
2. **Probe-only by default**. CLI commands must not write under `state/` unless an explicit subcommand or `--write`/non-`--dry-run` invocation says so.
3. **JSON Lines for append-only**. Logs and headers append to `.jsonl`; index/snapshot files use `.json`.
4. **UTC-stamped subdirs**. Time-bucketed evidence uses `YYYY-MM-DD/` UTC.
5. **Container-friendly**. The whole `~/.heiwa/` tree is mountable into a container so the same binary works on host and in Docker.
6. **Machine-local capability truth**. `machine.json` is refreshed at app boot,
   but its `device_id` and `installed_at` never rotate. Credentials and live
   process handles are never written into it or replicated.

## Container Mount

```bash
docker run --rm \
  -v "$HOME/.heiwa:/root/.heiwa" \
  ghcr.io/strategizing/heiwa:dev app runtime status --json
```

The container ships with `HEIWA_HOME=/root/.heiwa` and `HEIWA_DEFAULT_POLICY=local-only-no-side-effects`.

## Distribution

GitHub is the source of truth for source and binaries.

- Source: <https://github.com/Heiwa-Limited/heiwa-universe>
- Container: `ghcr.io/heiwa-limited/heiwa:<tag>` (currently `linux/amd64`, built from `apps/heiwa_shell/Dockerfile`)
- Binary releases: GitHub Releases on tag push (see `.github/workflows/release.yml`)

## Filesystem Hygiene

Do not treat the entire `state/` subtree as recreatable. Read models and caches
can be rebuilt, but approvals, connector enrollment, calendar holds/receipts,
and other user-authored records are durable local truth.

Identity and connection anchors include:

- `accounts.json`
- `local-identity.json`
- `identity.json`
- `machine.json`
- `secrets/` (keychain references)
- `state/connectors/`
- `state/dispatch/approvals/decisions/`
- user-authored or receipt-bearing domain records under `state/`
- canonical journals under `evidence/`

`cache/`, logs, derived Lance indexes, health probes, and external-source read
models are rebuildable. There is no hosted backend that can restore unsynced
local identity, decisions, receipts, or evidence journals.
