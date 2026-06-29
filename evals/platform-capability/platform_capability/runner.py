"""Trigger a platform-capability eval run and report results.

Triggers a run via POST /v1/evals/{id}/runs, polls until completion, and prints
the aggregate summary plus a per-case pass/fail table.

Usage:
    python -m platform_capability.runner --eval-id eval_xxx
    python -m platform_capability.runner --eval-id eval_xxx --model claude-sonnet-4-6
    python -m platform_capability.runner --eval-id eval_xxx --tag safety
"""

from __future__ import annotations

import argparse
import sys
import time

from .api import EvalClient

TERMINAL = {"completed", "failed", "cancelled"}


def run(args: argparse.Namespace) -> None:
    client = EvalClient(base_url=args.base_url, api_key=args.api_key)

    run_obj = client.create_run(
        args.eval_id,
        model_override=args.model,
        filter_tags=[args.tag] if args.tag else None,
    )
    run_id = run_obj["id"]
    print(f"Triggered run {run_id} (eval {args.eval_id})")
    if args.model:
        print(f"  model override: {args.model}")

    last_status = None
    while True:
        run_obj = client.get_run(args.eval_id, run_id)
        status = run_obj.get("status")
        if status != last_status:
            print(f"  status: {status}")
            last_status = status
        if status in TERMINAL:
            break
        time.sleep(args.poll_interval)

    _print_report(run_obj)
    client.close()

    summary = run_obj.get("summary") or {}
    failed = summary.get("failed", 0) + summary.get("errored", 0)
    sys.exit(1 if (run_obj.get("status") != "completed" or failed) else 0)


def _print_report(run_obj: dict) -> None:
    summary = run_obj.get("summary") or {}
    print("\n=== Summary ===")
    if summary:
        print(f"  pass rate : {summary.get('pass_rate', 0):.0%}")
        print(
            f"  cases     : {summary.get('passed', 0)} passed, "
            f"{summary.get('failed', 0)} failed, {summary.get('errored', 0)} errored "
            f"of {summary.get('total', 0)}"
        )
        print(f"  avg score : {summary.get('avg_score', 0):.2f}")
        print(f"  avg turns : {summary.get('avg_turns', 0):.1f}")
    else:
        print("  (no summary available)")

    results = run_obj.get("results") or run_obj.get("case_results") or []
    if results:
        print("\n=== Per-case ===")
        for r in results:
            name = r.get("case_name") or r.get("eval_case_id") or "?"
            status = r.get("status", "?")
            score = r.get("score")
            score_str = f" score={score:.2f}" if isinstance(score, (int, float)) else ""
            mark = {"passed": "PASS", "failed": "FAIL"}.get(status, status.upper())
            print(f"  [{mark}] {name}{score_str}")


def main():
    parser = argparse.ArgumentParser(description="Run a platform-capability eval")
    parser.add_argument("--eval-id", required=True, help="Eval ID from the loader")
    parser.add_argument("--base-url", default=None, help="Everruns API base URL")
    parser.add_argument("--api-key", default=None, help="Everruns API key")
    parser.add_argument("--model", default=None, help="Model override for this run")
    parser.add_argument("--tag", default=None, help="Only run cases with this tag")
    parser.add_argument(
        "--poll-interval", type=float, default=10.0, help="Seconds between polls"
    )
    args = parser.parse_args()
    run(args)


if __name__ == "__main__":
    main()
