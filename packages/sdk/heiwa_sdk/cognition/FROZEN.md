# 🧊 FROZEN — db.py Quarantine Notice

**Status:** FROZEN as of 2026-02-16
**Reason:** `libs/heiwa_sdk/db.py` is a 2,688-line god object and the #1 maintenance risk.

## Rules

1. **NO new methods** may be added to `db.py`
2. **NO new features** may be implemented inside `db.py`
3. Bug fixes to existing methods are permitted but must be documented
4. New cognitive/reasoning logic goes in `libs/heiwa_sdk/cognition/`
5. Future domain modules (e.g., `proposals.py`, `nodes.py`, `alerts.py`) will be extracted from `db.py` when the system is stable

## Migration Path

```
db.py (FROZEN)
  ├── cognition/engine.py     ← Atomic Broadcast (MIGRATED)
  ├── cognition/reasoning/    ← ConfidenceGate (MIGRATED)
  │
  │ FUTURE EXTRACTIONS:
  ├── proposals.py            ← Proposal CRUD + state transitions
  ├── nodes.py                ← Node registration + liveness
  ├── alerts.py               ← Alert scanning + generation
  └── ticks.py                ← Tick cycle + RFC publishing
```

## Why

Modifying `db.py` risks cascading failures across the entire system. Every agent, every tick cycle, and every proposal flows through this single file. We stabilize the deployment pipeline first, then perform surgery.
