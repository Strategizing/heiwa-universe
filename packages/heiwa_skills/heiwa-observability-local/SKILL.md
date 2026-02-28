---
name: heiwa-observability-local
description: "Local observability for Heiwa Codex workflows: structured workflow events, summaries, redaction, and operator diagnostics across profiles. Use when inspecting or improving Codex-driven workflow visibility and event hygiene."
---

# Heiwa Observability Local

This skill covers local observability for the Heiwa Codex operating layer.

## Storage

- Events: `/Users/dmcgregsauce/.codex/heiwa/observability/events/`
- Reports: `/Users/dmcgregsauce/.codex/heiwa/observability/reports/`
- Index: `/Users/dmcgregsauce/.codex/heiwa/observability/index.sqlite`

## Commands

```bash
/Users/dmcgregsauce/.codex/heiwa/bin/heiwax observe tail --profile heiwa-limited
/Users/dmcgregsauce/.codex/heiwa/bin/heiwax observe summary --since 24h
```

Prefer summarizing trends and failures, not dumping raw logs.
