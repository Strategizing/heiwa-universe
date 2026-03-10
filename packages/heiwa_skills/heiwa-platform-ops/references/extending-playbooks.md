# Extending Provider Playbooks

Use this file when adding support for a new CLI/provider in this skill.

## Add a New Provider

1. Create a new reference file in `references/` (for example `vercel-cli.md`).
2. Add a probe profile in `scripts/cli_probe.py`:
   - aliases
   - version command(s)
   - non-interactive auth/status command(s)
   - install hint
   - notes
3. Update `SKILL.md` provider list to point to the new reference file.
4. Validate the skill.

## What to Capture in Each Provider Reference

- Version/auth/context checks
- Common read-only inspection commands
- Common mutating commands (deploy, secret set, config updates)
- Validation commands
- Known failure modes
- Heiwa-specific file paths or conventions (if any)

## Source of Truth Strategy

- Prefer local repo docs and config first.
- Prefer local CLI `--help` output second.
- Use official provider docs for syntax changes or new features.
- Avoid copying large docs into the skill; keep references concise and procedural.

## Command Quality Rules

- Prefer commands that are non-interactive.
- Prefer commands with machine-readable output.
- Include explicit environment/project selection when possible.
- Avoid examples that leak secrets or require unsafe defaults.
