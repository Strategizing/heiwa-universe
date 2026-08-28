# Receipts

> Tokens are operator-truth. Currency is a presentation overlay. Receipts are the authority that connects them.

## Status (2026-08-27)

Spec status: **partial**.

Implemented in `crates/heiwa_receipts/`: schema and migrations, insert and query, env/agent/model rollups, rate-table loading, actual and counterfactual cost computation, and a tamper-evident SHA-256 hash chain with `verify_chain`.

Not implemented: prompt bodies, write-ahead catch-up, and a `heiwa receipts` command. `heiwa cost` is the shipped read surface. `counterfactual_cost_cad` is stored but not yet computed automatically.

Retired: the hosted mirror. The backend pivot of 2026-07-15 removed SpacetimeDB; there is no remote authority plane and no receipt mirror. `header()` still returns the redacted subset a future export path may carry, and nothing consumes it. Any replacement export must clear the redaction policy before it ships.

See [`HEIWA.md`](https://github.com/Heiwa-Limited/heiwa-universe/blob/main/HEIWA.md) for the current-vs-target capability matrix.

## What a receipt here does not prove

This document specifies **call receipts**: evidence that Heiwa spent something on a model or tool call. A call receipt is not evidence that anything happened outside Heiwa.

A successful model call can produce no effect at all. An effect can occur even when the caller loses the response and records an error. Proof that a file was written, a branch published, a message sent, or a payment made is a separate noun — an **Effect Receipt** — with its own target, idempotency posture, verification, uncertainty, and compensation semantics.

The Effect Receipt does not exist yet. It is named here so the gap is visible rather than papered over, and separating the two is publication gate 1 of the [Work Continuity Triple design](https://github.com/Heiwa-Limited/heiwa-universe/blob/main/docs/superpowers/specs/2026-08-27-heiwa-work-continuity-triple-design.md). Until it exists, no surface may present a call receipt as proof of an external effect.

## Why this schema

Every operator view the cockpit ships — by lane, by agent, by model, by day, by session — is a `SUM(...) GROUP BY ...` rollup over receipts. Build the schema once, the views compose for free. Adding a new presentation (currency, time window, agent slice) does not require new storage.

The schema is small on purpose. Twelve fields. Everything else is a function over them.

## Schema

| Field                     | Type      | Source                  | Notes                                                                                      |
| ------------------------- | --------- | ----------------------- | ------------------------------------------------------------------------------------------ |
| `id`                      | ULID      | runtime                 | sortable, monotonic                                                                        |
| `at`                      | timestamp | runtime                 | UTC, ISO-8601                                                                              |
| `env`                     | enum      | runtime                 | `local` / `oauth` / `api`                                                                  |
| `provider`                | string    | runtime                 | `ollama` / `claude-code` / `codex` / `gemini` / `openrouter` / ...                         |
| `model`                   | string    | runtime                 | provider-namespaced model id (e.g. `claude-sonnet-4-6`)                                    |
| `agent`                   | string    | caller                  | which operator agent invoked this — `coding`, `strategy`, `trading`, `summarise`, ...      |
| `tokens_in`               | int       | provider                | prompt tokens                                                                              |
| `tokens_out`              | int       | provider                | completion tokens                                                                          |
| `latency_ms`              | int       | runtime                 | end-to-end including queue + network                                                       |
| `actual_cost_cad`         | decimal   | rate-table × tokens     | always CAD as base unit; presentation in other currencies is a divide-at-read-time overlay |
| `counterfactual_cost_cad` | decimal   | api-rate-table × tokens | what the same tokens would cost on the metered API lane for this model                     |
| `session_id`              | ULID      | runtime                 | groups related receipts                                                                    |
| `parent_id`               | ULID?     | runtime                 | optional — for sub-call attribution (agent-spawned receipts point at the parent receipt)   |

### Why `env × provider × model × agent`

Four dimensions, each independently meaningful:

- **`env`** — where it ran. Determines whether incremental cost is zero (local, oauth) or metered (api).
- **`provider`** — who served it. Provider-level rollups answer "is Claude Code worth the sub?" or "is OpenRouter still our cheapest API gateway?"
- **`model`** — which variant. Per-model rollups expose unused capability ("we paid for opus but rarely call it") or unexpected escalation ("trading agent kept hitting gpt-4o because Sonnet was rate-limited").
- **`agent`** — what asked for it. Per-agent rollups attribute spend to the operator's _intent_ rather than the routing decision.

Removing any one dimension collapses a question the operator can already ask today. Adding more (region, sandbox, lease) is reserved for when a real use case demands it.

### Why CAD as base unit

CAD is the operator's local currency. Storing the value in the operator's locale keeps the schema rate-table-independent — currency conversion is presentation. Changing the displayed currency does not migrate data.

The `_cad` suffix in the field name makes the base unit explicit, so a "renamed to USD" mistake is visible at the column rather than inferred from a rate table.

## Storage

- **Primary**: SQLite at `~/.heiwa/receipts.db`. Written synchronously at the end of every cost-bearing call. The runtime never blocks completing a call on a receipt write failing — receipts have a write-ahead log that catches up on next start.
- **Prompt bodies**: `~/.heiwa/prompts/<id>.txt` (gzip). Stored alongside receipts so drill-down can show the actual prompt without piping it through the network. Optional to record completions (`--record-output`).
- **Remote mirror**: none. Local SQLite is the durable truth. The hosted plane that once received receipt headers was retired 2026-07-15.

### The exportable subset

`CallReceipt::header()` returns the redacted subset that a sharing boundary may carry. Nothing consumes it today; it exists so that future fields cannot leak by default.

```
{
  id, at, env, provider, model,
  agent,        // optional — operators may redact agent attribution before export
  tokens_in, tokens_out, latency_ms,
  actual_cost_cad, counterfactual_cost_cad,
  schema_version
}
```

### What never crosses a sharing boundary

- Prompt content
- Completion text
- Provider tokens, OAuth refresh tokens, API keys
- Local model weights
- Operator filesystem paths

Enforced by construction: `CallReceipt::header()` builds the exportable subset field by field, so a new field on the row does not reach a sharing boundary unless someone adds it there deliberately. `crates/heiwa_stdb` no longer exists — the reducer signatures this paragraph once named went with the backend pivot, and the boundary moved into the type.

## Cost calculation

`actual_cost_cad` and `counterfactual_cost_cad` are computed at receipt-write time from `~/.heiwa/rates.toml`. Two cost columns per receipt — the difference is Heiwa's value made arithmetic.

### Rate table example

```toml
# ~/.heiwa/rates.toml — operator-editable, synced from upstream weekly
synced_at = "2026-05-25T11:00:00Z"

[rates.api.openrouter."claude-3.7-sonnet"]
input_per_mtok_cad  = 4.05
output_per_mtok_cad = 20.25
# actual == counterfactual for api entries
counterfactual.input_per_mtok_cad  = 4.05
counterfactual.output_per_mtok_cad = 20.25

[rates.oauth.claude-code."claude-sonnet-4-6"]
# sub-backed, zero incremental cost
input_per_mtok_cad  = 0.0
output_per_mtok_cad = 0.0
# counterfactual = what the SAME model costs through Anthropic API
counterfactual.input_per_mtok_cad  = 4.05
counterfactual.output_per_mtok_cad = 20.25

[rates.local.ollama."qwen3.5:9b"]
input_per_mtok_cad  = 0.0
output_per_mtok_cad = 0.0
# counterfactual = nearest hosted equivalent as a fairness proxy
counterfactual.input_per_mtok_cad  = 0.27
counterfactual.output_per_mtok_cad = 0.81
counterfactual.note = "Mistral 7B pricing tier as proxy for 9B-class model"
```

### Why the counterfactual lives in the rate table

Operators tune their own counterfactual policy. Some want **strict same-model** counterfactual (Claude via OAuth vs. Claude via API). Others want **cost-equivalent capability** counterfactual (Ollama vs. nearest hosted equivalent). The runtime stores both inputs and one chosen counterfactual — switching the policy regenerates the column without touching the receipt's actual cost.

### Stale-rate handling

The runtime surfaces a `rates · stale 14d` fiducial when `now() - synced_at > 7 days`. Past 30 days, cost columns render with a `?` suffix to flag that the displayed number is using rates older than the operator's policy.

## Query patterns

```sql
-- Today, by environment
SELECT env,
       SUM(tokens_in + tokens_out) AS tokens,
       SUM(actual_cost_cad)        AS spent_cad,
       SUM(counterfactual_cost_cad) - SUM(actual_cost_cad) AS saved_cad
FROM receipts
WHERE at >= date('now')
GROUP BY env
ORDER BY tokens DESC;

-- Last 7d, by agent
SELECT agent,
       COUNT(*)                                              AS calls,
       SUM(tokens_in + tokens_out)                           AS tokens,
       SUM(actual_cost_cad)                                  AS spent_cad,
       SUM(counterfactual_cost_cad)                          AS would_have_spent_cad
FROM receipts
WHERE at >= date('now', '-7 days')
GROUP BY agent
ORDER BY tokens DESC;

-- Counterfactual delta per model — surfaces which model saves the most
SELECT model,
       SUM(actual_cost_cad)         AS actual,
       SUM(counterfactual_cost_cad) AS counterfactual,
       SUM(counterfactual_cost_cad) - SUM(actual_cost_cad) AS savings
FROM receipts
GROUP BY model
ORDER BY savings DESC;

-- Latency p50/p95 per provider — for routing decisions
SELECT provider,
       COUNT(*)                                                AS calls,
       PERCENTILE_CONT(0.50) WITHIN GROUP (ORDER BY latency_ms) AS p50_ms,
       PERCENTILE_CONT(0.95) WITHIN GROUP (ORDER BY latency_ms) AS p95_ms
FROM receipts
WHERE at >= date('now', '-1 days')
GROUP BY provider;
```

## CLI surface

```bash
heiwa cost                            # today's actual + counterfactual + savings
heiwa cost --by agent                 # group rollup
heiwa cost --since 7d --by model      # 7-day window, grouped by model
heiwa cost --ccy USD                  # one-shot currency override
heiwa headroom                        # OAuth window state per provider
heiwa receipts                        # paginated list, most recent first
heiwa receipts show <id>              # drill-down with prompt
heiwa receipts grep "trading"         # search receipts by tag/agent/prompt-substring
heiwa receipts export --redact > out.jsonl   # share without prompts
```

Default presentation reads the operator's locale; `~/.heiwa/config.toml` sets a different default if needed.

## Privacy boundary

| Surface                                     | What is visible                                                                                       |
| ------------------------------------------- | ----------------------------------------------------------------------------------------------------- |
| Local SQLite (`~/.heiwa/receipts.db`)       | full record                                                                                           |
| Local prompts (`~/.heiwa/prompts/<id>.txt`) | prompt body (gzip)                                                                                    |
| `CallReceipt::header()` (exportable subset) | id, at, env, provider, model, optional-redacted agent, tokens, latency, both cost columns             |
| `heiwa receipts export` (planned)           | full record minus prompt body                                                                         |
| `heiwa receipts export --redact` (planned)  | tokens + cost only; no agent, no model, no provider — useful for audit attestation without disclosure |

## Schema versioning

The schema is versioned in `~/.heiwa/receipts.db`'s `schema_version` row. Migrations land in `crates/heiwa_receipts/migrations/`. The exportable subset always includes `schema_version` so a consumer on an older version does not silently drop new fields.

### Compatibility rules

- **Additive changes** (new optional columns): minor bump. Older readers see `NULL`.
- **Type changes** or **renames**: major bump. Both reader and writer must upgrade.
- **Removals**: major bump. The removed column is tombstoned in the migration, not dropped, so an older exported header still parses.

## Open questions

These are deliberately not resolved in this spec — they will land as separate proposals as use cases concretize.

- **Multi-currency receipts** — operators with mixed-locale billing (Canadian operator on a US-billed Anthropic sub) may want a `billed_in` enum alongside `_cad`. Current answer: store in CAD, let the operator's accounting layer reconcile.
- **Token-class refinement** — `tokens_in` does not distinguish cached vs. uncached input tokens (relevant for prompt-caching pricing). Either subdivide the field or add `tokens_cached_in`.
- **Receipt tags** — free-form operator labels (`tag=#post-mortem-2026-04-21`) would let the operator group receipts across agents without changing the schema. Likely a sidecar table.
- **Cross-device session continuity** — when an operator continues a session on a second device, do receipts share `session_id` or use a `continuation_of` field? With no remote authority plane, this now depends on the Continuation contract in the Work Continuity design rather than on a shared write path.

## Where to next

- [Publishing Pipeline](../publishing.md) — how the runtime and docs reach operators
- [Security](../security.md) — disclosure policy and runtime threat model around receipt content
- [Operator Runbook](../operator-runbook.md) — day-to-day operation including receipt querying
