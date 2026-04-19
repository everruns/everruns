"""Score SWE-bench predictions using the official Docker harness, then write scores back.

Flow:
  1. Read predictions.jsonl (from runner export or manual file)
  2. Run swebench Docker harness → produces results
  3. PATCH scores back to Everruns via bulk write-back API

Usage:
    # Score and write back
    python -m swe_bench.scorer --eval-id eval_xxx --run-id evalrun_xxx --predictions predictions.jsonl

    # Dry run (score locally, don't write back)
    python -m swe_bench.scorer --predictions predictions.jsonl --dry-run

    # Skip Docker harness, use pre-computed results
    python -m swe_bench.scorer --eval-id eval_xxx --run-id evalrun_xxx --results results.json
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path

from .api import EvalClient

SWEBENCH_DATASET = "princeton-nlp/SWE-bench_Lite"


def run_swebench_harness(predictions_path: str, *, max_workers: int = 4) -> dict[str, dict]:
    """Run the official SWE-bench Docker evaluation harness.

    Returns mapping of instance_id → {resolved: bool, ...}.
    """
    with tempfile.TemporaryDirectory() as tmpdir:
        output_dir = Path(tmpdir) / "output"
        output_dir.mkdir()

        cmd = [
            sys.executable, "-m", "swebench.harness.run_evaluation",
            "--predictions_path", predictions_path,
            "--swe_bench_tasks", SWEBENCH_DATASET,
            "--log_dir", str(output_dir / "logs"),
            "--testbed", str(output_dir / "testbed"),
            "--skip_existing",
            "--timeout", "900",
            "--max_workers", str(max_workers),
        ]

        print(f"Running: {' '.join(cmd)}")
        result = subprocess.run(cmd, capture_output=True, text=True)

        if result.returncode != 0:
            print(f"SWE-bench harness stderr:\n{result.stderr}", file=sys.stderr)
            if result.returncode != 0:
                print(f"Warning: harness exited with code {result.returncode}", file=sys.stderr)

        report_path = output_dir / "logs" / "report.json"
        if not report_path.exists():
            candidates = list((output_dir / "logs").rglob("report.json"))
            if candidates:
                report_path = candidates[0]
            else:
                print("Error: no report.json found in harness output", file=sys.stderr)
                print(f"Output dir contents: {list(output_dir.rglob('*'))}", file=sys.stderr)
                sys.exit(1)

        with open(report_path) as f:
            report = json.load(f)

    return report


def load_results_file(path: str) -> dict[str, dict]:
    """Load pre-computed SWE-bench results from a JSON file."""
    with open(path) as f:
        return json.load(f)


def build_scores(resolved: bool) -> list[dict]:
    if resolved:
        return [{"pass": True, "value": 1.0, "reason": "All FAIL_TO_PASS tests pass, PASS_TO_PASS tests preserved"}]
    return [{"pass": False, "value": 0.0, "reason": "FAIL_TO_PASS tests did not pass"}]


def write_scores_back(
    client: EvalClient,
    eval_id: str,
    run_id: str,
    report: dict[str, dict],
) -> None:
    """Write SWE-bench harness results back to Everruns."""
    run = client.get_run(eval_id, run_id)
    results = run.get("results", [])

    if not results:
        print("Error: run has no results to update", file=sys.stderr)
        sys.exit(1)

    resolved_ids = set()
    for instance_id, instance_report in report.items():
        if isinstance(instance_report, dict) and instance_report.get("resolved", False):
            resolved_ids.add(instance_id)
        elif isinstance(instance_report, bool) and instance_report:
            resolved_ids.add(instance_id)

    bulk_items = []
    matched = 0
    for r in results:
        case_name = r.get("case_name", "")
        if not case_name:
            continue

        resolved = case_name in resolved_ids
        bulk_items.append({
            "result_id": r["id"],
            "scores": build_scores(resolved),
            "status": "passed" if resolved else "failed",
        })
        matched += 1

    if not bulk_items:
        print("Error: no results matched report instance IDs", file=sys.stderr)
        sys.exit(1)

    print(f"Writing scores for {matched} results ({len(resolved_ids)} resolved)...")

    metadata = {
        "scorer": "swebench-docker-harness",
        "scored_at": datetime.now(timezone.utc).isoformat(),
        "dataset": SWEBENCH_DATASET,
        "resolved_count": len(resolved_ids),
        "total_count": matched,
    }

    resp = client.bulk_update_scores(eval_id, run_id, results=bulk_items, metadata=metadata)
    print(f"Scores written. Response contains {len(resp) if isinstance(resp, list) else 'bulk'} results.")


def print_report_summary(report: dict[str, dict]) -> None:
    total = len(report)
    resolved = sum(
        1 for v in report.values()
        if (isinstance(v, dict) and v.get("resolved", False)) or (isinstance(v, bool) and v)
    )
    print(f"\nSWE-bench Results: {resolved}/{total} resolved ({resolved/total:.1%})" if total else "No results")


def run(args: argparse.Namespace) -> None:
    if args.results:
        report = load_results_file(args.results)
    elif args.predictions:
        report = run_swebench_harness(args.predictions, max_workers=args.max_workers)
    else:
        print("Error: provide either --predictions or --results", file=sys.stderr)
        sys.exit(1)

    print_report_summary(report)

    if args.dry_run:
        print("\nDry run — skipping score write-back")
        if args.output:
            with open(args.output, "w") as f:
                json.dump(report, f, indent=2)
            print(f"Report saved to {args.output}")
        return

    if not args.eval_id or not args.run_id:
        print("Error: --eval-id and --run-id required for score write-back", file=sys.stderr)
        sys.exit(1)

    client = EvalClient(base_url=args.base_url, api_key=args.api_key)
    write_scores_back(client, args.eval_id, args.run_id, report)
    client.close()

    print("Done.")


def main():
    parser = argparse.ArgumentParser(description="Score SWE-bench predictions and write back to Everruns")
    parser.add_argument("--eval-id", default=None, help="Eval ID (eval_xxx)")
    parser.add_argument("--run-id", default=None, help="Run ID (evalrun_xxx)")
    parser.add_argument("--base-url", default=None, help="Everruns API base URL")
    parser.add_argument("--api-key", default=None, help="Everruns API key")
    parser.add_argument("--predictions", default=None, help="Path to predictions.jsonl")
    parser.add_argument("--results", default=None, help="Path to pre-computed results JSON (skip harness)")
    parser.add_argument("--max-workers", type=int, default=4, help="Docker harness parallelism")
    parser.add_argument("--dry-run", action="store_true", help="Score locally without writing back")
    parser.add_argument("--output", "-o", default=None, help="Save report to file")
    args = parser.parse_args()
    run(args)


if __name__ == "__main__":
    main()
