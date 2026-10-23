#!/usr/bin/env python3
"""Offline scale and fixed synthetic-query evaluation for MCP catalogs.

Run from this directory with `python3 evaluate.py`. The script needs only the
Python standard library and the sibling `fixture/server.py`; it makes no API
calls. Sizes include the 50-tool smoke catalog, 600-tool baseline, and a 2,000-
tool catalog. Serialized sizes are UTF-8 bytes; chars/4 is shown separately as
an explicitly rough token estimate, never as a measured token count.
"""
from __future__ import annotations

import argparse
import importlib.util
import hashlib
import json
import math
import statistics
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from retrieval import rank_bm25, rank_everruns  # noqa: E402


def load_cases(path: Path = HERE / "cases.jsonl") -> list[dict[str, Any]]:
    with path.open(encoding="utf-8") as stream:
        return [json.loads(line) for line in stream if line.strip() and not line.lstrip().startswith("#")]


def load_fixture_module() -> Any:
    path = HERE / "fixture" / "server.py"
    spec = importlib.util.spec_from_file_location("mcp_scale_fixture", path)
    if not spec or not spec.loader:
        raise RuntimeError(f"cannot load fixture module at {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def load_fixture() -> Callable[..., list[dict[str, Any]]]:
    return load_fixture_module().build_catalog


def compact_bytes(value: Any) -> int:
    return len(json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode("utf-8"))


def schema_bytes(tools: list[dict[str, Any]]) -> dict[str, int | float]:
    full = compact_bytes(tools)
    stubs = []
    for tool in tools:
        stub = dict(tool)
        stub["inputSchema"] = {"type": "object", "additionalProperties": True}
        stubs.append(stub)
    stub_bytes = compact_bytes(stubs)
    return {
        "full_catalog_bytes": full,
        "full_catalog_chars_div_4_estimate": round(full / 4, 1),
        "deferred_stub_catalog_bytes": stub_bytes,
        "deferred_stub_chars_div_4_estimate": round(stub_bytes / 4, 1),
        "schema_reduction_percent": round(100 * (full - stub_bytes) / full, 2) if full else 0.0,
    }


def _percentile(values: list[float], p: float) -> float:
    if not values:
        return 0.0
    ordered = sorted(values)
    pos = (len(ordered) - 1) * p
    lo = math.floor(pos)
    hi = math.ceil(pos)
    return ordered[lo] + (ordered[hi] - ordered[lo]) * (pos - lo)


def latency_ms(search: Callable[[list[dict[str, Any]], str], list[dict[str, Any]]],
               tools: list[dict[str, Any]], queries: list[str], repeats: int) -> dict[str, float]:
    # Warm once, then collect individual in-process retrieval timings.
    for query in queries:
        search(tools, query)
    samples: list[float] = []
    for _ in range(repeats):
        for query in queries:
            started = time.perf_counter_ns()
            search(tools, query)
            samples.append((time.perf_counter_ns() - started) / 1_000_000)
    return {"p50_ms": round(_percentile(samples, .50), 4),
            "p95_ms": round(_percentile(samples, .95), 4),
            "samples": len(samples)}


def ranking_metrics(cases: list[dict[str, Any]], tools: list[dict[str, Any]],
                    search: Callable[[list[dict[str, Any]], str], list[dict[str, Any]]]) -> dict[str, Any]:
    positives = [c for c in cases if c.get("expected")]
    negatives = [c for c in cases if not c.get("expected")]
    hits = {k: 0 for k in (1, 5, 8)}
    reciprocal_ranks = []
    misses: list[dict[str, str]] = []
    for case in positives:
        expected = set(case["expected"] if isinstance(case["expected"], list) else [case["expected"]])
        names = [tool.get("name") for tool in search(tools, case["query"])]
        ranks = [i + 1 for i, name in enumerate(names) if name in expected]
        first = min(ranks) if ranks else None
        reciprocal_ranks.append(1 / first if first else 0.0)
        for k in hits:
            hits[k] += int(first is not None and first <= k)
        if first is None:
            misses.append({"id": case["id"], "query": case["query"], "expected": ",".join(sorted(expected))})
    false_positives = [c["id"] for c in negatives if search(tools, c["query"])]
    count = len(positives)
    return {
        "positive_cases": count,
        "negative_cases": len(negatives),
        "recall_at_1": round(hits[1] / count, 4) if count else 0,
        "recall_at_5": round(hits[5] / count, 4) if count else 0,
        "recall_at_8": round(hits[8] / count, 4) if count else 0,
        "mrr": round(statistics.mean(reciprocal_ranks), 4) if reciprocal_ranks else 0,
        "negative_false_positive_count": len(false_positives),
        "negative_false_positive_rate": round(len(false_positives) / len(negatives), 4) if negatives else 0,
        "misses": misses,
    }


def schema_problem(schema: Any, path: str = "inputSchema") -> str | None:
    """Validate the JSON Schema subset emitted by the deterministic fixture."""
    if not isinstance(schema, dict):
        return f"{path} must be an object"
    kind = schema.get("type")
    if kind not in {"object", "string", "integer"}:
        return f"{path}.type must be object, string, or integer"
    if kind == "object":
        props = schema.get("properties", {})
        required = schema.get("required", [])
        if not isinstance(props, dict) or not isinstance(required, list):
            return f"{path} has malformed properties/required"
        if any(key not in props for key in required):
            return f"{path}.required references a missing property"
        for key, value in props.items():
            problem = schema_problem(value, f"{path}.properties.{key}")
            if problem:
                return problem
    if "enum" in schema and (not isinstance(schema["enum"], list) or not schema["enum"]):
        return f"{path}.enum must be a non-empty array"
    if kind == "integer" and "minimum" in schema and "maximum" in schema:
        if schema["minimum"] > schema["maximum"]:
            return f"{path}.minimum exceeds maximum"
    return None


def all_required_recall_at_8(cases: list[dict[str, Any]], tools: list[dict[str, Any]],
                              search: Callable[[list[dict[str, Any]], str], list[dict[str, Any]]]) -> dict[str, Any]:
    required_cases = [case for case in cases if case.get("required_tools")]
    if not required_cases:
        return {"cases": 0, "score": None}
    scores = []
    for case in required_cases:
        required = set(case["required_tools"])
        found = {tool.get("name") for tool in search(tools, case["query"])[:8]}
        scores.append(len(required & found) / len(required))
    return {"cases": len(scores), "score": round(statistics.mean(scores), 4),
            "all_tools_recalled_cases": sum(score == 1 for score in scores)}


def gateway_schema_bytes() -> int | None:
    # Root owns this module and may add it after this script lands. Import lazily
    # so the offline evaluator remains usable without the gateway implementation.
    try:
        from gateway import GATEWAY_TOOLS  # type: ignore
    except (ImportError, AttributeError):
        return None
    return compact_bytes(GATEWAY_TOOLS)


def run(sizes: list[int], repeats: int = 25) -> dict[str, Any]:
    fixture = load_fixture_module()
    build_catalog = fixture.build_catalog
    cases = load_cases()
    reports = []
    for size in sizes:
        # Fixture accepts tools_count; retain the alias for callers that adopt the
        # earlier tool_count spelling.
        try:
            tools = build_catalog(tools_count=size)
        except TypeError:
            tools = build_catalog(tool_count=size)
        names = {tool["name"] for tool in tools}
        schema_errors = [tool.get("name", "<unnamed>") for tool in tools
                         if schema_problem(tool.get("inputSchema"))]
        if schema_errors:
            raise ValueError(f"invalid fixture inputSchema for tools: {schema_errors[:10]}")
        invalid_expected = [case["id"] for case in cases
                            if case.get("expected") and any(name not in names for name in
                                (case["expected"] if isinstance(case["expected"], list) else [case["expected"]]))]
        invalid_required = [case["id"] for case in cases
                            if any(name not in names for name in case.get("required_tools", []))]
        if invalid_expected or invalid_required:
            raise ValueError(f"query targets missing from {size}-tool fixture: {invalid_expected + invalid_required}")
        by_name = {tool["name"]: tool for tool in tools}
        bad_examples = []
        for case in cases:
            if "expected_arguments" not in case:
                continue
            tool_name = case.get("expected_argument_tool", case.get("expected"))
            error = fixture._validate(by_name[tool_name]["inputSchema"], case["expected_arguments"])
            if error:
                bad_examples.append(f"{case['id']}: {error}")
        if bad_examples:
            raise ValueError("invalid example arguments: " + "; ".join(bad_examples))
        queries = [case["query"] for case in cases]
        aliases_present = any(tool.get("aliases") for tool in tools)
        searches = {
            "everruns_keyword": lambda catalog, query: rank_everruns(catalog, query),
            "bm25": lambda catalog, query: rank_bm25(catalog, query),
        }
        if aliases_present:
            searches["bm25_with_aliases"] = lambda catalog, query: rank_bm25(catalog, query, include_aliases=True)
        methods = {}
        for name, search in searches.items():
            methods[name] = {
                **ranking_metrics(cases, tools, search),
                "latency": latency_ms(search, tools, queries, repeats),
                "all_required_recall_at_8": all_required_recall_at_8(cases, tools, search),
            }
        report = {
            "catalog_size": len(tools),
            "catalog_order": "fixture deterministic SHA-256 order by tool name",
            "serialized_schemas": schema_bytes(tools),
            "aliases_present": aliases_present,
            "retrieval": methods,
        }
        gw_bytes = gateway_schema_bytes()
        if gw_bytes is not None:
            report["constant_gateway_search_call_schema_bytes"] = gw_bytes
        reports.append(report)
    revision = subprocess.run(["git", "-C", str(HERE.parents[1]), "rev-parse", "HEAD"],
                              check=True, capture_output=True, text=True).stdout.strip()
    digest_files = {
        "cases.jsonl": HERE / "cases.jsonl",
        "fixture/server.py": HERE / "fixture" / "server.py",
        "retrieval.py": HERE / "retrieval.py",
        "gateway.py": HERE / "gateway.py",
    }
    return {
        "evaluation": "mcp-scale-offline-v1",
        "generated_at_utc": datetime.now(timezone.utc).isoformat(),
        "git_revision": revision,
        "sha256": {name: hashlib.sha256(path.read_bytes()).hexdigest()
                   for name, path in digest_files.items()},
        "cases_file": "cases.jsonl",
        "case_status": "fixed synthetic queries; not a held-out study",
        "case_count": len(cases),
        "metric_note": "Positive queries have one intended target, so recall@k equals target hit rate. Recall/MRR measure retrieval only; no model task success is inferred.",
        "size_reports": reports,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sizes", nargs="+", type=int, default=[50, 600, 2000])
    parser.add_argument("--repeats", type=int, default=25, help="latency samples per query")
    parser.add_argument("--output", type=Path, help="write JSON report to this path (stdout otherwise)")
    args = parser.parse_args()
    result = run(args.sizes, args.repeats)
    output = json.dumps(result, indent=2, ensure_ascii=False) + "\n"
    if args.output:
        args.output.write_text(output, encoding="utf-8")
    else:
        sys.stdout.write(output)


if __name__ == "__main__":
    main()
