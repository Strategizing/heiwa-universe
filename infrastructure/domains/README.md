# Heiwa Domain Bootstrap

This folder contains the initial domain bootstrap manifest for `heiwa.ltd`.

## Source of Truth
- Strategy source: `HEIWA_DOMAIN_PLAN.md`
- Bootstrap manifest: `infrastructure/domains/heiwa-ltd.bootstrap.json`

## Initial Domain Set
- `status.heiwa.ltd` -> public status site (Cloudflare Pages)
- `auth.heiwa.ltd` -> identity/auth edge
- `api.heiwa.ltd` -> swarm API ingress
- `docs.heiwa.ltd` -> docs surface

## Operator Notes
- Keep all provider credentials out of repo.
- Domain cutover should be approval-gated.
- Run health checks after each DNS or routing change.
