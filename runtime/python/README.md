# heiwa-sidecar

Python compatibility sidecar exposing diagnostics and compatibility operations over stdio.

## Wire protocol

- **Framing:** one JSON object per line on stdin / stdout (JSONL). stderr is free-form logs.
- **Request:** `{"id": str, "op": str, "args": {...}}`
- **Response:** `{"id": str, "status": "ok", "result": any}` or `{"id": str, "status": "err", "code": str, "message": str}`

## Built-in ops

| op           | purpose                                           |
| ------------ | ------------------------------------------------- |
| `health`     | Liveness probe. Returns `{"status": "ok"}`.       |
| `version`    | Sidecar, Python, and platform versions.           |
| `check_deps` | Probe importability of langgraph/optional llama_index. |
| `echo`       | Return `args` verbatim (wire-test helper).        |
| `shutdown`   | Reply and exit loop cleanly.                      |

Add new ops in `src/heiwa_sidecar/handlers.py` and register in `HANDLERS`.

## Local dev

```bash
cd runtime/python
uv sync --extra dev
uv run python -m heiwa_sidecar   # serves on stdin/stdout
uv run --extra dev python -m pytest # run tests
```

Use `python -m heiwa_sidecar` for subprocess integration; `heiwa-sidecar` is also provided for interactive debugging.

## Dependency boundary

`check_deps` reports whether LangGraph and an externally installed LlamaIndex
module are importable. Missing LlamaIndex produces `importable: false` under
the existing `llama_index` result key; all sidecar operations remain available.
The probe does not establish graph, model, or indexing execution capability.

The `llama` installation extra is retired: no sidecar operation used its APIs,
and it pulled an unpatched NLTK dependency into the shipped lockfile. Run
`uv sync --extra dev` to reconcile an existing development environment with
the reduced dependency graph. Reintroducing LlamaIndex requires a concrete
execution contract and verified dependencies.
