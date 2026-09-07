#!/usr/bin/env python3
"""Local check inventory and private, revision-bound verification receipts."""

from __future__ import annotations

import argparse
from dataclasses import dataclass
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import time


@dataclass(frozen=True)
class Check:
    name: str
    argv: tuple[str, ...]
    timeout_seconds: float = 1200
    deferred: str | None = None


def source_state(root: Path) -> dict:
    def git(*args: str) -> str:
        return subprocess.check_output(["git", "-C", str(root), *args], text=True).strip()

    return {
        "head": git("rev-parse", "HEAD"),
        "dirty": bool(git("status", "--porcelain=v1", "--untracked-files=all")),
    }


def now() -> str:
    return datetime.now(timezone.utc).isoformat()


def write_receipt(path: Path, receipt: dict) -> None:
    temporary = path.with_suffix(".tmp")
    with open(temporary, "w", encoding="utf-8", opener=lambda p, f: os.open(p, f, 0o600)) as stream:
        json.dump(receipt, stream, indent=2)
        stream.write("\n")
        stream.flush()
        os.fsync(stream.fileno())
    temporary.replace(path)


def stop_process(process: subprocess.Popen) -> None:
    """Stop only this check's owned process group, then reap its direct child."""
    def terminate(sig: int) -> None:
        try:
            if os.name == "posix":
                os.killpg(process.pid, sig)
            elif sig == signal.SIGTERM:
                process.terminate()
            else:
                process.kill()
        except ProcessLookupError:
            pass

    terminate(signal.SIGTERM)
    try:
        process.wait(timeout=2)
    except subprocess.TimeoutExpired:
        terminate(signal.SIGKILL)
        process.wait()
    if os.name == "posix":
        terminate(signal.SIGKILL)


def run_checks(root: Path, checks: list[Check], *, profile: str) -> tuple[int, Path]:
    root = root.resolve()
    started = source_state(root)
    parent = root / "private" / "verification"
    parent.mkdir(parents=True, exist_ok=True, mode=0o700)
    run_dir = Path(tempfile.mkdtemp(prefix="local-ci-", dir=parent))
    path = run_dir / "receipt.json"
    receipt = {
        "schema_version": 1, "kind": "local-ci", "profile": profile,
        "status": "running", "started_at": now(), "source_start": started,
        "checks": [], "errors": [],
    }
    write_receipt(path, receipt)
    print(f"Verification evidence: {path}", flush=True)
    interrupted = False
    try:
        for index, check in enumerate(checks):
            row = {"name": check.name, "status": "running"}
            receipt["checks"].append(row)
            if check.deferred is not None:
                row.update(status="deferred", reason=check.deferred)
                write_receipt(path, receipt)
                print(f"  {check.name}: DEFERRED ({check.deferred})", flush=True)
                continue

            row.update(log=f"{index:02d}.log", command=list(check.argv), started_at=now())
            write_receipt(path, receipt)
            print(f"  {check.name}: ", end="", flush=True)
            monotonic_start = time.monotonic()
            process = None
            environment = os.environ.copy()
            environment["HEIWA_VERIFICATION_LOG_DIR"] = str(run_dir / f"{index:02d}-details")
            log_path = run_dir / row["log"]
            with open(log_path, "w", encoding="utf-8", opener=lambda p, f: os.open(p, f, 0o600)) as log:
                try:
                    process = subprocess.Popen(
                        check.argv, cwd=root, env=environment, stdin=subprocess.DEVNULL,
                        stdout=log, stderr=subprocess.STDOUT, start_new_session=os.name == "posix",
                    )
                    row["exit_code"] = process.wait(timeout=check.timeout_seconds)
                    row["status"] = "passed" if row["exit_code"] == 0 else "failed"
                except subprocess.TimeoutExpired:
                    row.update(status="timed_out", exit_code=124)
                    log.write(f"\nCheck exceeded {check.timeout_seconds} seconds.\n")
                except (OSError, ValueError) as error:
                    row.update(status="failed", exit_code=127)
                    log.write(f"Unable to execute check: {error}\n")
                except KeyboardInterrupt:
                    row.update(status="interrupted", exit_code=130)
                    raise
                finally:
                    # A direct child exiting does not prove its work stopped.
                    # Reap its group before collecting final source identity.
                    if process is not None:
                        stop_process(process)
                    row["duration_seconds"] = round(time.monotonic() - monotonic_start, 3)
                    write_receipt(path, receipt)
            print(row["status"].upper(), flush=True)
            if row["status"] != "passed":
                with log_path.open(errors="replace") as log:
                    for _, line in zip(range(25), log):
                        print(f"      {line.rstrip()}")
    except KeyboardInterrupt:
        interrupted = True
        receipt["errors"].append("Verification interrupted; remaining checks were not executed.")
    except Exception as error:
        receipt["errors"].append(f"Verification runner failed: {type(error).__name__}: {error}")
    finally:
        try:
            receipt["source_end"] = source_state(root)
            if started["dirty"] or receipt["source_end"]["dirty"]:
                receipt["errors"].append("A clean source tree is required at both ends of verification.")
            if started["head"] != receipt["source_end"]["head"]:
                receipt["errors"].append("HEAD changed during verification.")
        except (OSError, subprocess.SubprocessError) as error:
            receipt["errors"].append(f"Unable to verify final source identity: {error}")
        executed = [row for row in receipt["checks"] if row["status"] != "deferred"]
        if not executed:
            receipt["errors"].append("No checks were executed.")
        passed = not receipt["errors"] and all(row["status"] == "passed" for row in executed)
        receipt["status"] = "interrupted" if interrupted else "passed" if passed else "failed"
        receipt["finished_at"] = now()
        write_receipt(path, receipt)
    print(f"LOCAL CHECKS {receipt['status'].upper()}. Receipt: {path}", flush=True)
    for error in receipt["errors"]:
        print(f"  {error}", flush=True)
    return (130 if interrupted else 0 if receipt["status"] == "passed" else 1), path


def check_inventory(root: Path, *, full: bool, python: str) -> list[Check]:
    checks = []

    def add(name: str, *argv: str) -> None:
        checks.append(Check(name, argv))

    if full:
        add("protoc (Lance prerequisite)", "protoc", "--version")
    add("cargo fmt", "cargo", "fmt", "--all", "--", "--check")
    add("Rust test inventory", "bash", "scripts/ci_rust_test_group.sh", "--check")
    rust_tests = ("cargo", "test", "--workspace", "--exclude", "heiwa-desktop", "--locked")
    add("Rust workspace tests", *rust_tests, "--no-default-features")
    if full:
        add("Lance tests", *rust_tests, "--features", "heiwa-shell/lance")
    allows = (
        "too_many_arguments", "new_without_default", "unnecessary_to_owned", "needless_range_loop",
        "approx_constant", "collapsible_if", "bool_assert_comparison", "type_complexity",
        "needless_borrows_for_generic_args", "unnecessary_unwrap",
    )
    add("strict Rust Clippy", "cargo", "clippy", "--workspace", "--exclude", "heiwa-desktop",
        "--locked", "--all-targets", "--no-default-features", "--",
        *(arg for name in allows for arg in ("-A", f"clippy::{name}")), "-D", "warnings")
    add("cargo machete", "cargo", "machete")
    add("npm ci", "npm", "ci", "--ignore-scripts")
    add("desktop npm ci", "npm", "ci", "--ignore-scripts", "--prefix", "apps/heiwa_app/desktop")
    add("web typecheck", "npm", "run", "typecheck")
    add("web lint", "npm", "run", "lint")
    add("Python tests", python, "-m", "pytest", "-q")
    add("product tests", "just", "test-product")
    add("Python sidecar", "bash", "scripts/check_python_sidecar.sh")
    add("strict docs build", "uv", "run", "--locked", "--extra", "docs", "python", "-m", "mkdocs", "build", "--strict")
    add("agent instruction sync", python, "scripts/sync_agents.py", "--check")
    add("local Python resolver", "bash", "scripts/tests/test_local_python_resolution.sh")
    add("Justfile Python override", "bash", "scripts/tests/test_just_python_override.sh")
    add("release source regression", "bash", "scripts/tests/test_release_workflow_source.sh")
    add("required CI results regression", "bash", "scripts/tests/test_ci_required_checks.sh")
    for script in (
        "check_agent_baseline", "check_backend_transition", "check_model_call_boundary",
        "check_release_metadata", "check_runtime_baseline", "verify_security", "check_machine_security",
        "check_heiwa_core_dockerfile", "check_workflow_pins", "check_public_installer",
    ):
        add(script, "bash", f"scripts/{script}.sh")
    for level in ("l0", "l1", "l2"):
        add(f"{level.upper()} acceptance", "bash", f"scripts/check_{level}_acceptance.sh")
    if full:
        add("native desktop certification", "bash", "scripts/check_native_desktop.sh")
    else:
        checks.append(Check("native desktop certification", (), deferred="run --full for native tests, Clippy, and release build; required independently in PR CI"))
    a1 = "scripts/check_work_fabric_a1_acceptance.sh"
    if (root / a1).is_file():
        add("Work Fabric A1 acceptance", "bash", a1)
    else:
        checks.append(Check("Work Fabric A1 acceptance", (), deferred="gate not implemented; A1 remains incomplete"))
    return checks


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--full", action="store_true", help="include Lance and native desktop certification")
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    python = os.environ.get("HEIWA_PYTHON", sys.executable)
    os.environ.update(CARGO_PROFILE_DEV_DEBUG="0", CARGO_INCREMENTAL="0", HEIWA_PYTHON=python)
    os.environ.setdefault("HEIWA_PYTEST", f"{python} -m pytest")
    os.umask(0o077)

    def interrupted(signum, frame):
        raise KeyboardInterrupt

    signal.signal(signal.SIGTERM, interrupted)
    checks = check_inventory(root, full=args.full, python=python)
    return run_checks(root, checks, profile="full" if args.full else "default")[0]


if __name__ == "__main__":
    raise SystemExit(main())
