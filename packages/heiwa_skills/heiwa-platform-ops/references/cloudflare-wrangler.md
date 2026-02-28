# Cloudflare Wrangler Playbook

Use this file for Cloudflare CLI workflows (Workers, Pages, data products) with a discovery-first approach.

## Version and Auth

Start with:

```bash
wrangler --version
wrangler whoami
wrangler --help
```

If command syntax differs, trust local `wrangler --help` output over memory.

## Project Context

Before deploying, inspect local config files in the repo:

- `wrangler.toml`
- `wrangler.json`
- `wrangler.jsonc`

Confirm:

- Worker/Pages project name
- Account/environment mapping
- Bindings (KV, D1, R2, Queues, etc.)
- Environment-specific sections

## Common Workflows (Examples)

Use these as patterns, then confirm exact flags with local help:

```bash
wrangler deploy
wrangler tail
wrangler secret put <NAME>
wrangler d1 --help
wrangler kv --help
wrangler pages --help
```

For deploy debugging:

1. Run `wrangler deploy` (or scoped deploy command).
2. Run `wrangler tail` for live logs when applicable.
3. Validate the endpoint with a smoke request.

## Safe Practices

- Do not paste secrets inline; use `secret put` flows or environment files.
- Confirm account and environment before deploying to production.
- Prefer read-only list/status/tail commands before mutating config.
- Capture the deployment ID/version output when available for rollback context.

## When to Escalate

Escalate to official docs if:

- Wrangler subcommands differ materially from expected syntax
- A product feature is newly released or beta
- Auth mode or account scoping is unclear
