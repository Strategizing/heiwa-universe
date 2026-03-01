# HEARTBEAT.md - Sovereign Mesh Monitor

- **Sovereign Health:** Check CPU/RAM via `heiwa status`. If CPU > 50% or RAM > 90%, notify Devon on Discord.
- **Monorepo Audit:** Once per day, run a non-destructive audit of `~/heiwa`. Check for lockfile drifts.
- **Discord Presence:** Verify status is `dnd` and activity is `iClaw @ Macbook`.
- **iMessage (Low-Level Burst):** Check `imsg` connectivity. If SIP is still blocking `chat.db`, prioritize Discord for high-fidelity or suggest BlueBubbles.
- **Mesh Node Discovery:** Check for other active nodes in the swarm.

If all is normal, reply `HEARTBEAT_OK`.
