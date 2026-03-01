# System Initialization Directive: Project Heiwa Architect

**To: iClaw / OpenClaw Autonomous Agent**
**Target Model:** `google-gemini-cli/gemini-3-pro-preview` (or equivalent ultra-high context engine)
**Objective:** Execute the entire Heiwa Universe SpacetimeDB architecture upgrade autonomously.

## Context & Strategy
You have been allocated a massively expanded context window (2,000,000 tokens) and an extended tool-iteration limit (150 iterations) to tackle a complex, multi-phase system overhaul without losing state or context.

Your absolute source of truth is located at:
`~/heiwa/HEIWA_UNIVERSE_BLUEPRINT.md`

All local documentation and SDKs you need are cached at:
`~/heiwa/docs_and_deps/`

## Execution Protocol
You are to execute the `HEIWA_UNIVERSE_BLUEPRINT.md` sequentially. 
Do not skip phases. Execute each phase, test and validate its compilation/configuration, and then move to the next.

**Your Immediate First Steps:**
1. **Read and ingest** `~/heiwa/HEIWA_UNIVERSE_BLUEPRINT.md` in its entirety.
2. **Execute Phase 0:** Scaffold the monorepo structure within `~/heiwa`.
3. **Execute Phase 1:** Write the Rust SpacetimeDB schema and reducers in `apps/heiwa_hub/src/lib.rs`.
4. Compile the database module (`spacetime publish`) and generate the client bindings.
5. Pause only if you encounter a catastrophic compilation failure you cannot resolve by consulting `~/heiwa/docs_and_deps/`.

**Rules of Engagement:**
- DO NOT rely on external web searches for SpacetimeDB or MCP documentation. Search the local `~/heiwa/docs_and_deps/` directory first to ensure compatibility.
- Ensure all Rust reducers adhere to the mathematical isolation constraints described in the Blueprint.
- Ensure all Docker sandboxes utilize Rootless mode constraints.

Proceed immediately.