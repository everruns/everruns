"""Trigger an eval run and poll until completion.

Exports artifacts as predictions.jsonl when done.

Usage:
    python -m swe_bench.runner --eval-id eval_xxx
    python -m swe_bench.runner --eval-id eval_xxx --model claude-sonnet-4-20250514
    python -m swe_bench.runner --eval-id eval_xxx --poll-interval 30
"""

from __future__ import annotations

import argparse
import json
import sys
import time

from .api import EvalClient

TERMINAL_STATUSES = {"completed", "failed", "cancelled"}


def poll_run(client: EvalClient, eval_id: str, run_id: str, interval: int = 15) -> dict:
    while True:
        run = client.get_run(eval_id, run_id)
        status = run["status"]
        summary = run.get("summary")

        results = run.get("results", [])
        completed = sum(1 for r in results if r["status"] in ("passed", "failed", "errored", "timeout"))
        total = len(results)

        if summary:
            print(
                f"\r  status={status}  progress={completed}/{total}  "
                f"pass_rate={summary.get('pass_rate', 0):.1%}",
                end="",
                flush=True,
            )
        else:
            print(f"\r  status={status}  progress={completed}/{total}", end="", flush=True)

        if status in TERMINAL_STATUSES:
            print()
            return run

        time.sleep(interval)


def export_artifacts(client: EvalClient, eval_id: str, run_id: str, output_path: str) -> int:
    ndjson = client.get_run_artifacts_ndjson(eval_id, run_id)
    lines = [line for line in ndjson.strip().split("\n") if line.strip()]

    with open(output_path, "w") as f:
        f.write(ndjson)

    return len(lines)


def print_summary(run: dict) -> None:
    summary = run.get("summary")
    if not summary:
        print("No summary available.")
        return

    print(f"\n{'=' * 50}")
    print(f"Run {run.get('public_id', run.get('id', '?'))}")
    print(f"{'=' * 50}")
    print(f"  Status:       {run['status']}")
    print(f"  Total:        {summary['total']}")
    print(f"  Passed:       {summary['passed']}")
    print(f"  Failed:       {summary['failed']}")
    print(f"  Errored:      {summary['errored']}")
    print(f"  Pass Rate:    {summary['pass_rate']:.1%}")
    print(f"  Avg Score:    {summary['avg_score']:.3f}")
    print(f"  Avg Turns:    {summary['avg_turns']:.1f}")
    print(f"  Avg Latency:  {summary['avg_latency_ms']}ms")
    print(f"  Input Tokens: {summary['total_input_tokens']:,}")
    print(f"  Output Tokens:{summary['total_output_tokens']:,}")


def run(args: argparse.Namespace) -> None:
    client = EvalClient(base_url=args.base_url, api_key=args.api_key)

    target = None
    if args.harness:
        target = {"type": "session", "harness_name": args.harness}

    print(f"Triggering eval run for {args.eval_id}...")
    run_resp = client.create_run(
        args.eval_id,
        target=target,
        model_override=args.model,
    )
    run_id = run_resp.get("public_id", run_resp.get("id"))
    print(f"Run created: {run_id}")

    print("Polling for completion...")
    final_run = poll_run(client, args.eval_id, run_id, interval=args.poll_interval)

    print_summary(final_run)

    if final_run["status"] == "completed":
        output_path = args.output or f"predictions-{run_id}.jsonl"
        count = export_artifacts(client, args.eval_id, run_id, output_path)
        print(f"\nExported {count} predictions to {output_path}")
        print(f"Next: python -m swe_bench.scorer --eval-id {args.eval_id} --run-id {run_id} --predictions {output_path}")
    else:
        print(f"\nRun did not complete successfully (status={final_run['status']})", file=sys.stderr)
        sys.exit(1)

    client.close()


def main():
    parser = argparse.ArgumentParser(description="Trigger and monitor an Everruns eval run")
    parser.add_argument("--eval-id", required=True, help="Eval ID (eval_xxx)")
    parser.add_argument("--base-url", default=None, help="Everruns API base URL")
    parser.add_argument("--api-key", default=None, help="Everruns API key")
    parser.add_argument("--model", default=None, help="Model override for this run")
    parser.add_argument("--harness", default=None, help="Harness name override for this run")
    parser.add_argument("--poll-interval", type=int, default=15, help="Seconds between status polls")
    parser.add_argument("--output", "-o", default=None, help="Output path for predictions JSONL")
    args = parser.parse_args()
    run(args)


if __name__ == "__main__":
    main()
