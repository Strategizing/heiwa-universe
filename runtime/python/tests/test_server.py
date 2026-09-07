"""Stdio loop + dispatch tests. No real ML imports triggered."""

from __future__ import annotations

import asyncio
import io
import json
import sys
import types

import pytest

from heiwa_sidecar import __version__
from heiwa_sidecar.handlers import dispatch
from heiwa_sidecar.protocol import Request, err
from heiwa_sidecar.server import serve


@pytest.mark.asyncio
async def test_health_and_version_round_trip() -> None:
    inp = io.StringIO(
        json.dumps({"id": "1", "op": "health"}) + "\n"
        + json.dumps({"id": "2", "op": "version"}) + "\n"
    )
    out = io.StringIO()
    code = await serve(inp, out)
    assert code == 0

    lines = [json.loads(line) for line in out.getvalue().splitlines()]
    assert lines[0] == {"id": "1", "status": "ok", "result": {"status": "ok"}}
    assert lines[1]["id"] == "2"
    assert lines[1]["status"] == "ok"
    assert lines[1]["result"]["sidecar"] == __version__


@pytest.mark.asyncio
async def test_unknown_op_returns_err() -> None:
    inp = io.StringIO(json.dumps({"id": "9", "op": "nope"}) + "\n")
    out = io.StringIO()
    await serve(inp, out)
    resp = json.loads(out.getvalue())
    assert resp == {
        "id": "9",
        "status": "err",
        "code": "unknown_op",
        "message": "no handler for op 'nope'",
    }


@pytest.mark.asyncio
async def test_malformed_json_returns_err() -> None:
    inp = io.StringIO("{not-json\n")
    out = io.StringIO()
    await serve(inp, out)
    resp = json.loads(out.getvalue())
    assert resp["status"] == "err"
    assert resp["code"] == "parse_error"


@pytest.mark.asyncio
async def test_shutdown_stops_loop() -> None:
    frames = [
        json.dumps({"id": "1", "op": "echo", "args": {"x": 1}}),
        json.dumps({"id": "2", "op": "shutdown"}),
        json.dumps({"id": "3", "op": "echo", "args": {"x": 2}}),
    ]
    inp = io.StringIO("\n".join(frames) + "\n")
    out = io.StringIO()
    await serve(inp, out)

    lines = [json.loads(line) for line in out.getvalue().splitlines()]
    assert len(lines) == 2
    assert lines[0]["result"] == {"x": 1}
    assert lines[1]["result"] == {"shutting_down": True}


@pytest.mark.asyncio
async def test_validation_failure_is_typed_error() -> None:
    inp = io.StringIO(json.dumps({"op": "health"}) + "\n")  # missing id
    out = io.StringIO()
    await serve(inp, out)
    resp = json.loads(out.getvalue())
    assert resp["status"] == "err"
    assert resp["code"] == "parse_error"


@pytest.mark.asyncio
@pytest.mark.parametrize("llama_installed", [False, True])
async def test_check_deps_reports_optional_module_availability(
    monkeypatch: pytest.MonkeyPatch, llama_installed: bool
) -> None:
    langgraph = types.ModuleType("langgraph")
    langgraph.__version__ = "fixture-langgraph"
    llama = types.ModuleType("llama_index") if llama_installed else None
    if llama is not None:
        llama.__version__ = "fixture-llama"
    monkeypatch.setitem(sys.modules, "langgraph", langgraph)
    monkeypatch.setitem(sys.modules, "llama_index", llama)

    resp = await dispatch(Request(id="x", op="check_deps"))
    assert resp.status == "ok"
    assert resp.result["langgraph"] == {
        "importable": True,
        "version": "fixture-langgraph",
    }
    if llama_installed:
        assert resp.result["llama_index"] == {
            "importable": True,
            "version": "fixture-llama",
        }
    else:
        assert resp.result["llama_index"]["importable"] is False
        assert "ModuleNotFoundError" in resp.result["llama_index"]["error"]


@pytest.mark.asyncio
async def test_handler_exception_is_caught() -> None:
    async def boom(_req: Request):
        raise RuntimeError("boom")

    from heiwa_sidecar import handlers

    handlers.HANDLERS["boom"] = boom
    try:
        resp = await dispatch(Request(id="1", op="boom"))
        assert resp.status == "err"
        assert resp.code == "handler_error"
        assert "boom" in resp.message
    finally:
        del handlers.HANDLERS["boom"]


def test_err_helper_populates_code_and_message() -> None:
    resp = err("42", code="x", message="y")
    assert resp.status == "err"
    assert resp.code == "x"
    assert resp.message == "y"


def test_asyncio_run_integration() -> None:
    """End-to-end via asyncio.run, mirroring the `run()` entry point."""
    inp = io.StringIO(json.dumps({"id": "1", "op": "health"}) + "\n")
    out = io.StringIO()
    asyncio.run(serve(inp, out))
    assert json.loads(out.getvalue())["status"] == "ok"
