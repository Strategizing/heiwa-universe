---
name: heiwa-project-profile
description: Load, validate, and use Heiwa project profiles from `/Users/dmcgregsauce/.codex/heiwa/profiles/*.yaml` for consistent repo/provider/Figma context. Use when a workflow should be profile-driven instead of hardcoded.
---

# Heiwa Project Profile

Use this skill whenever a task spans GitHub/Railway/Figma/runtime context and should resolve targets from a profile instead of ad hoc commands.

## Workflow

1. List profiles: `heiwax profile list`
2. Validate target profile: `heiwax profile validate <profile_id>`
3. Use profile-derived identifiers for all audits/mutations
4. Emit evidence with profile ID and target refs

## Commands

```bash
/Users/dmcgregsauce/.codex/heiwa/bin/heiwax profile list
/Users/dmcgregsauce/.codex/heiwa/bin/heiwax profile validate heiwa-limited
```
