# Current Capability Truth

## Supported now

- **Installed `heiwa` CLI/cockpit**: supported operator surface
- **MCP/tool registry**: supported integration surface with scoped local tools
- **Connector manifests**: validated manifest surface with negative audit coverage
- **HTTP API**: supported public-safe runtime ingress where hosted services are deployed
- **Docs and release artifacts**: supported GitHub-native publication surfaces

## Supported architecture claims

- The application is written for N users. `crates/heiwa_config::HeiwaPaths`
  (ConfigRoot) is the single resolver for per-user state, and
  `scripts/check_l0_acceptance.sh` fails on any independent home/state-root
  resolution or hardcoded operator identity in runtime code. The check greps
  source rather than proving absence by construction, so it is a guard
  against regression, not a proof.
- A user supplies one API key and the application works with no provider CLI
  installed: direct-API adapters for the Anthropic, OpenAI, and Google
  families run alongside the CLI adapters.
  `apps/heiwa_shell/tests/fresh_install.rs` proves this by running the built
  `heiwa` binary with an emptied `PATH`, `HEIWA_BIN_DIRS` empty so no system
  directory is probed, a temp state root, and no reachable local runtime. It
  asserts the model's text reaches stdout, the request carried the user's
  key exactly once, and the run registered no provider-CLI account. Keys
  resolve from the OS keychain first and the provider's conventional
  environment variable otherwise, so a container or CI runner with no
  keychain works; the harness uses a distinct account id so it reads no real
  keychain entry.
  Scope: the harness covers the Anthropic wire format. OpenAI and Google have
  unit-level wire coverage but are not driven through the binary.
- Provider failure is a routing constraint: `heiwa_provider::health` reports
  which accounts are usable and why one was skipped, and a zero-provider
  install opens with actionable guidance rather than an error. Routing reads
  that projection: `AccountRegistry::routable_models` filters on health, not
  on stored status, so a CLI seat or local runtime whose executable is gone
  offers no route instead of failing the turn on an OS error. A connected
  account with an empty inventory is reported with a way out rather than
  silently yielding no models.
  Credential rejection is classified from the HTTP status, never by matching
  text in a provider's response body.
- First run happens inside the application. `heiwa setup` establishes a local
  per-installation identity and reports every remaining gap with the action
  that closes it, exiting non-zero while incomplete; the desktop shows the
  same projection as an overlay and can close the identity gap in-window.
  `heiwa_identity::onboarding` is the only place readiness is decided, so the
  two surfaces cannot disagree. `apps/heiwa_shell/tests/first_run.rs` drives
  the shipped binary from an empty state root to a completed turn.
  Identity is local and per-installation and contacts no server; whether it
  is also account-backed is the open D1 fork and is not decided.
- A fresh install completes a turn with no credentials at all. Verified by
  walking an empty state root with the shipped binary: `heiwa setup` mints the
  local identity, discovery finds a local runtime if one is present, and
  `heiwa ask` answers — no API key, no account, no network. Where no local
  runtime exists, the same command reports the provider gap with the actions
  that close it (AD-13).
- Apple Calendar begins disconnected for every fresh Heiwa profile. An
  explicit in-app or `heiwa connect apple-calendar --authorize` action binds
  enrollment to the local installation and device; before that, resource
  discovery returns no calendar names and does not probe Calendar.app.
  Once connected, Calendar reads from the machine, not the cloud. `heiwa calendar sync` pulls
  events from the user's own Calendar.app through a JXA metadata bridge into
  a snapshot under the config root; `/api/v1/calendar/summary` serves it and
  the Calendar surface renders it. The same Mac lane discovers exact writable
  calendars and can stage an event from CLI or Heiwa.app; only a T2 approval
  invokes Calendar.app, after which the external id and connector receipt
  replay from local JSONL. Verified against a real calendar, then removed by
  exact marker/id. Google Calendar remains expansion work and reports
  `needs_auth`.
- Mail reads from the machine, not the cloud. `heiwa mail scan` pulls sender,
  subject, date, and read state from the user's own Mail.app — never a body —
  into a snapshot under the config root, and the Mail surface renders it. No
  OAuth, no IMAP credentials, nothing leaves the machine. Sending is not
  built; the surface says so. Gmail and other remote mailboxes remain L3.
- The desktop shell is a SolidJS component layer: eleven surface modules behind a
  `SurfaceModule` contract over a tokenized design system, with the operator
  stream seam (`store.ts` / `client.ts` / `types.ts`) preserved unmodified.
  Calendar can connect/disconnect and stage an exact Apple event; Approvals can
  approve or deny that pending write through the same immutable executor as the
  CLI.
- The installed runtime is the current product center of gravity.
- DREX routing, provider/session/protocol crates, execution scopes, tool leases, and receipts are the live runtime spine.
- Local JSONL is the canonical evidence plane; Lance is the derived local recall index.
- GitHub Actions, Pages, and Releases are the current repo-native validation and publication path.
- Cloudflare is optional support infrastructure for public edge needs; hosted services do not define the default operator experience.
- Public status is event-first when exposed, with HTTP diagnostics as fallback.

## Not presented as complete

- Discord as a required ingress surface
- iMessage as a productized ingress surface
- broad computer-use automation
- `Heiwa.app` as a fully native desktop runtime
- live connector read models behind Finance and the broader Social surfaces
- the Browser surface as an actionable, approval-gated automation surface; it
  is an iframe until the L4 runtime-owned browser lands
- cross-device evidence sync or a hosted state backbone
- `heiwa-limited` as an active product target
- experimental canvases as part of the supported stack
- placeholder agent personas as productized capabilities
- full provider-normalized multi-turn tool calling across every provider
- executable connector breadth beyond the Mac-first Apple Calendar lane
- the mesh as a working fabric. `crates/heiwa_mesh` gives this machine a node
  identity, a signed and hash-chained envelope frame, and an expiring
  capability advertisement, all provable on one machine. There is no peer
  transport, no pairing, and no replication: an enrolled node has no peers and
  cannot reach another device. `heiwa mesh status` and the Home machine
  perspective both state that, and a mesh state that cannot be read is
  reported as `unknown` rather than as `local_only`.

## Evidence rule

README, MkDocs pages, and the static web shell should all agree on this boundary. CI exists to prevent public claims from drifting ahead of verified surfaces.
