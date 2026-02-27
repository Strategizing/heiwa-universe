# Heiwa.ltd Domain Strategy

The `heiwa.ltd` domain serves as the public and administrative gateway to the Heiwa Swarm.

## 📡 1. Public Status Page (`status.heiwa.ltd`)
- **Technology**: Cloudflare Pages + Workers.
- **Purpose**: Real-time heartbeat of the mesh.
- **Data Source**: A Worker script that polls the Railway Cloud HQ and Macbook Node status via NATS (secured by a proxy).
- **UX**: A minimalist, high-tech dashboard showing "Cloud HQ: Online", "Macbook GPU: Attached", and "Swarm Latency: 45ms".

## 🔐 2. Identity & Auth (`auth.heiwa.ltd`)
- **Purpose**: SSO for any future web dashboards and agent-to-agent secure handshake.
- **Mechanism**: OAuth2 flow integrated with the Discord identity.

## 🛠️ 3. Swarm API (`api.heiwa.ltd`)
- **Purpose**: Trigger tasks from external sources (e.g., GitHub webhooks, personal mobile app).
- **Action**: Proxies requests into the `heiwa.tasks.ingress` NATS subject.

## 📖 4. Documentation (`docs.heiwa.ltd`)
- **Purpose**: Public/Private wiki for the Heiwa Canon.
- **Action**: Automatically synced from `heiwa/runtime/docs`.

# Next Steps
1.  Deploy the initial `status.heiwa.ltd` page using the existing `wrangler.toml`.
2.  Implement a NATS-to-HTTP proxy on Railway to feed the status page.
3.  Enable "Zero-Command" Discord interaction by strictly enforcing the `IntentNormalizer`.
