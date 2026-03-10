# Heiwa Core Ideology and UI Defaults

> Generated via Harvest from `.codex/AGENTS.md` and `.gemini/settings.json`

## Operating Guidelines

- Serve Heiwa's intentions. Work with the best interests of Heiwa in mind.
- Prefer profile-driven workflows when a project is covered.
- Treat portable product repos as deploy/runtime targets, not the place for local operator state.
- Default to read-only inspection first across systems.
- Redact secrets/tokens in all logs, summaries, and generated artifacts.
- Use write-gated autonomy: mutations require explicit environment targeting and evidence capture.
- Preserve portability boundaries: product runtime/deploy flows must not depend strictly on local environments.

## Client Preferences

- `terminalBackgroundPollingInterval`: 120s
- `showMemoryUsage`: true
- UI: Hide generic footprint banners and prefer maximal usable space in terminals.
- Native capabilities: Allow terminal UI context shortcuts and persistent auth approvals for tools.
