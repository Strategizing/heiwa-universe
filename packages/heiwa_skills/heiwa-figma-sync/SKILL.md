---
name: heiwa-figma-sync
description: Inspect Figma Make architecture designs, compare against Heiwa runtime/profile topology, and generate human-in-the-loop Figma sync packets (prompt + datasets + diff checklist). Use when architecture visuals drift from reality or after architecture-affecting changes.
---

# Heiwa Figma Sync

Use this skill to keep Figma architecture visuals aligned with Heiwa's actual topology. Direct in-place editing is not assumed.

## Workflow

1. Validate the profile (`heiwax profile validate ...`)
2. Inspect Figma via Figma MCP (read-only) when needed
3. Generate a sync packet (`heiwax sync figma --profile ...`)
4. Review `design_diff.md` and `acceptance_checklist.md`
5. Paste `figma_ai_prompt.txt` into Figma AI / Make

## Commands

```bash
/Users/dmcgregsauce/.codex/heiwa/bin/heiwax sync figma --profile heiwa-limited
```
