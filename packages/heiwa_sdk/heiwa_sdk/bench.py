from __future__ import annotations

from dataclasses import asdict, dataclass
import json
from pathlib import Path
from typing import Any

from heiwa_protocol.routing import BrokerRouteRequest

from .cells import HeiwaCellCatalog
from .heiwaclaw import HeiwaClawGateway


@dataclass(slots=True)
class BenchFailure:
    suite: str
    case: str
    field: str
    expected: Any
    actual: Any

    def to_dict(self) -> dict[str, Any]:
        return asdict(self)


class HeiwaBench:
    """Unified release-gate runner for route, gateway, and cell selection behavior."""

    def __init__(self, root_dir: Path | None = None) -> None:
        self.root = root_dir or Path(__file__).resolve().parents[3]
        self.suites_dir = self.root / "config" / "swarm" / "benchmarks"
        self.gateway = HeiwaClawGateway(self.root)
        self.cells = HeiwaCellCatalog(self.root)

    def _suite_files(self, suite: str | None = None) -> list[Path]:
        if suite:
            return [self.suites_dir / f"{suite}.json"]
        return sorted(self.suites_dir.glob("*.json"))

    @staticmethod
    def _resolve_field(data: dict[str, Any], dotted_path: str) -> Any:
        current: Any = data
        for token in str(dotted_path or "").split("."):
            if not token:
                continue
            if not isinstance(current, dict):
                return None
            current = current.get(token)
        return current

    def _run_routing_case(self, case: dict[str, Any]) -> tuple[dict[str, Any], list[BenchFailure]]:
        from heiwa_hub.cognition.enrichment import BrokerEnrichmentService

        request = BrokerRouteRequest.from_payload(case.get("request") or {})
        enrichment = BrokerEnrichmentService()
        result = enrichment.enrich(request)
        dispatch = self.gateway.resolve(result)
        actual = {"route": result.to_dict(), "dispatch": dispatch.to_dict()}
        failures: list[BenchFailure] = []
        for field, expected in dict(case.get("expect") or {}).items():
            observed = self._resolve_field(actual, field)
            if observed != expected:
                failures.append(
                    BenchFailure(
                        suite="routing_matrix",
                        case=str(case.get("name") or "unnamed"),
                        field=field,
                        expected=expected,
                        actual=observed,
                    )
                )
        return actual, failures

    def _run_cells_case(self, case: dict[str, Any]) -> tuple[dict[str, Any], list[BenchFailure]]:
        actual = self.cells.recommend(str(case.get("prompt") or ""))
        failures: list[BenchFailure] = []
        for field, expected in dict(case.get("expect") or {}).items():
            observed = self._resolve_field(actual, field)
            if observed != expected:
                failures.append(
                    BenchFailure(
                        suite="cells_catalog",
                        case=str(case.get("name") or "unnamed"),
                        field=field,
                        expected=expected,
                        actual=observed,
                    )
                )
        return actual, failures

    def _run_suite(self, suite_file: Path) -> dict[str, Any]:
        payload = json.loads(suite_file.read_text(encoding="utf-8"))
        suite_name = str(payload.get("suite") or suite_file.stem)
        evaluator = str(payload.get("evaluator") or suite_name)
        results: list[dict[str, Any]] = []
        failures: list[BenchFailure] = []

        for case in list(payload.get("cases") or []):
            if evaluator == "routing_matrix":
                actual, case_failures = self._run_routing_case(case)
            elif evaluator == "cells_catalog":
                actual, case_failures = self._run_cells_case(case)
            else:
                actual = {}
                case_failures = [
                    BenchFailure(
                        suite=suite_name,
                        case=str(case.get("name") or "unnamed"),
                        field="evaluator",
                        expected=evaluator,
                        actual="unsupported",
                    )
                ]

            results.append({"name": case.get("name", "unnamed"), "ok": not case_failures, "actual": actual})
            failures.extend(case_failures)

        return {
            "suite": suite_name,
            "description": payload.get("description", ""),
            "ok": not failures,
            "cases": results,
            "failures": [failure.to_dict() for failure in failures],
        }

    def run(self, suite: str | None = None) -> dict[str, Any]:
        suite_files = self._suite_files(suite)
        missing = [str(path) for path in suite_files if not path.exists()]
        if missing:
            return {
                "ok": False,
                "suite": suite,
                "error": f"Benchmark suite not found: {missing[0]}",
                "results": [],
                "failures": [],
            }

        results = [self._run_suite(path) for path in suite_files]
        failures = [failure for result in results for failure in result.get("failures", [])]
        total_cases = sum(len(result.get("cases", [])) for result in results)
        passed_cases = sum(1 for result in results for case in result.get("cases", []) if case.get("ok"))
        return {
            "ok": not failures,
            "suite": suite or "all",
            "total_cases": total_cases,
            "passed_cases": passed_cases,
            "failed_cases": total_cases - passed_cases,
            "results": results,
            "failures": failures,
        }

    def to_json(self, suite: str | None = None) -> str:
        return json.dumps(self.run(suite=suite), indent=2)
