# Railway Infrastructure Initialization Guide

This document tracks the target state for Heiwa's Cloud HQ on Railway.

## Components
1. **heiwa-cloud-hq** (Main Python Worker & Server)
2. **Postgres** (Persistent DB for Discord state & memories)
3. **NATS** (Message bus for agents)

## Bootstrapping a New Environment
If starting from scratch, execute these commands via the Railway CLI:

```bash
railway init --name heiwa_hub

# Add dependencies
railway add -d postgres
railway add --service nats --image nats:latest

# Set Variables for HQ
railway variables --set 'DATABASE_URL=${{Postgres.DATABASE_URL}}' --service heiwa-cloud-hq
railway variables --set 'NATS_URL=nats://nats.railway.internal:4222' --service heiwa-cloud-hq

# Link custom domain
railway domain link api.heiwa.ltd --service heiwa-cloud-hq
railway domain link auth.heiwa.ltd --service heiwa-cloud-hq
```

## Volumes & Persistence
Ensure the `Postgres` service has a mounted volume (usually automatic) to prevent Amnesia Mode resets.
