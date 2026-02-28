# Acceptance Checklist

- [ ] Domain edge layer includes `status/auth/api/docs` under `heiwa.ltd`.
- [ ] `status.heiwa.ltd` and `docs.heiwa.ltd` are shown as Cloudflare Pages targets.
- [ ] `auth.heiwa.ltd` and `api.heiwa.ltd` are shown as Railway edge targets with Cloudflare protection.
- [ ] Architecture map distinguishes public read-only web from privileged control-plane operations.
- [ ] Identity-routing layer includes `domain-web-initializer` and default fallback identity.
- [ ] OpenClaw and PicoClaw are represented as orchestrators, not sources of truth.
- [ ] Portability boundary note is present (no runtime dependency on `~/.codex/heiwa`).
- [ ] Human reviewer confirms diagram matches `infrastructure/domains/heiwa-ltd.bootstrap.json`.
