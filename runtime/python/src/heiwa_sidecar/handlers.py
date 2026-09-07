"""Op handlers dispatched from the stdio server.

Each op is a small async function. Ecosystem imports are dependency
diagnostics only; graph and indexing execution are not implemented here.
`health` and `version` do not import the ML libraries.
"""

from __future__ import annotations

import importlib
import platform
import sys
from typing import Any, Awaitable, Callable

from . import __version__
from .protocol import ErrResponse, OkResponse, Request, err, ok

Handler = Callable[[Request], Awaitable[OkResponse | ErrResponse]]


async def op_health(req: Request) -> OkResponse | ErrResponse:
    return ok(req.id, {"status": "ok"})


async def op_version(req: Request) -> OkResponse | ErrResponse:
    return ok(
        req.id,
        {
            "sidecar": __version__,
            "python": sys.version.split()[0],
            "platform": platform.platform(),
        },
    )


async def op_check_deps(req: Request) -> OkResponse | ErrResponse:
    """Report importability of known ecosystem modules, not execution capability.

    LlamaIndex is an external optional probe, not a sidecar dependency.
    Keep its result key for callers that already consume this diagnostic.
    """
    results: dict[str, Any] = {}
    for name in ("langgraph", "llama_index"):
        try:
            mod = importlib.import_module(name)
            version = getattr(mod, "__version__", None)
            results[name] = {"importable": True, "version": version}
        except Exception as exc:
            results[name] = {"importable": False, "error": repr(exc)}
    return ok(req.id, results)


async def op_echo(req: Request) -> OkResponse | ErrResponse:
    return ok(req.id, req.args)


async def op_shutdown(req: Request) -> OkResponse | ErrResponse:
    return ok(req.id, {"shutting_down": True})


async def op_vault_resolve(req: Request) -> OkResponse | ErrResponse:
    """Reject the retired Python state-backend credential path.

    Provider credentials are resolved by the owner runtime. The Python sidecar
    must never shell out to a retired backend or receive provider secrets.
    """
    return err(
        req.id,
        code="backend_retired",
        message="vault_resolve moved to the Rust runtime; Python sidecar access is retired",
    )


async def op_vault_decrypt(req: Request) -> OkResponse | ErrResponse:
    """Decrypt a ciphertext string using the InstanceVault."""
    ciphertext = req.args.get("ciphertext")
    if not ciphertext:
        return err(req.id, code="missing_args", message="ciphertext is required")

    try:
        from heiwa_sdk.vault import InstanceVault

        vault = InstanceVault()
        plaintext = vault.decrypt(ciphertext)
        return ok(req.id, {"plaintext": plaintext})
    except Exception as exc:
        return err(req.id, code="vault_error", message=repr(exc))


HANDLERS: dict[str, Handler] = {
    "health": op_health,
    "version": op_version,
    "check_deps": op_check_deps,
    "echo": op_echo,
    "shutdown": op_shutdown,
    "vault_resolve": op_vault_resolve,
    "vault_decrypt": op_vault_decrypt,
}


async def dispatch(req: Request) -> OkResponse | ErrResponse:
    handler = HANDLERS.get(req.op)
    if handler is None:
        return err(req.id, code="unknown_op", message=f"no handler for op '{req.op}'")
    try:
        return await handler(req)
    except Exception as exc:
        return err(req.id, code="handler_error", message=repr(exc))
