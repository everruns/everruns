"""Load SWE-bench Lite dataset into an Everruns eval.

Creates one Eval with 300 EvalCases (or --limit N for a subset).
Each case instructs the agent to fix the issue and write a patch file.

Usage:
    python -m swe_bench.loader                    # full 300 cases
    python -m swe_bench.loader --limit 2          # integration smoke test
    python -m swe_bench.loader --limit 10 --tag pilot
"""

from __future__ import annotations

import argparse
import json
import sys
import textwrap

from datasets import load_dataset

from .api import EvalClient

DATASET = "princeton-nlp/SWE-bench_Lite"
SPLIT = "test"
PATCH_PATH = "/workspace/fix.patch"

MAX_TURNS = 40
TIMEOUT_SECONDS = 900

# Two representative instances for integration testing
INTEGRATION_INSTANCES = [
    "astropy__astropy-12907",
    "django__django-11179",
]


def build_conversation(instance: dict) -> list[dict]:
    problem = instance["problem_statement"]
    repo = instance["repo"]
    base_commit = instance["base_commit"]
    repo_name = repo.split("/")[-1]

    prompt = textwrap.dedent(f"""\
        You are a software engineer working on the {repo} repository.

        <problem_statement>
        {problem}
        </problem_statement>

        <instructions>
        1. Create a Daytona sandbox and clone {repo} into it
        2. Checkout commit {base_commit}
        3. Read the relevant source code to understand the bug
        4. Implement the fix
        5. Run `cd /home/daytona/{repo_name} && git diff > /tmp/fix.patch` in the sandbox
        6. Use `daytona_download_workspace` to download `/tmp/fix.patch` from the sandbox to `{PATCH_PATH}` in the session filesystem
        </instructions>

        Important: The sandbox filesystem and session filesystem are separate. After creating the patch inside the sandbox, you MUST use `daytona_download_workspace` to copy it to `{PATCH_PATH}` in the session filesystem. This is how your answer is collected.
    """)
    return [{"content": prompt}]


def build_case(instance: dict, *, tags: list[str] | None = None) -> dict:
    instance_id = instance["instance_id"]
    case: dict = {
        "name": instance_id,
        "description": f"SWE-bench Lite: {instance_id}",
        "conversation": build_conversation(instance),
        "artifacts": [{"name": "patch", "path": PATCH_PATH}],
        "scorers": [
            {"type": "file_contains", "path": PATCH_PATH, "text": "diff", "weight": 1.0},
        ],
        "max_turns": MAX_TURNS,
        "timeout_seconds": TIMEOUT_SECONDS,
    }
    if tags:
        case["tags"] = tags
    return case


def load_swebench_instances(limit: int | None = None, integration: bool = False) -> list[dict]:
    ds = load_dataset(DATASET, split=SPLIT)
    if integration:
        instances = [row for row in ds if row["instance_id"] in INTEGRATION_INSTANCES]
        if len(instances) != len(INTEGRATION_INSTANCES):
            found = {r["instance_id"] for r in instances}
            missing = set(INTEGRATION_INSTANCES) - found
            print(f"Warning: missing integration instances: {missing}", file=sys.stderr)
        return instances
    instances = list(ds)
    if limit:
        instances = instances[:limit]
    return instances


def run(args: argparse.Namespace) -> None:
    client = EvalClient(base_url=args.base_url, api_key=args.api_key)

    is_integration = args.integration
    instances = load_swebench_instances(
        limit=args.limit,
        integration=args.integration,
    )
    print(f"Loaded {len(instances)} SWE-bench instances")

    eval_tags = ["swe-bench"]
    if args.tag:
        eval_tags.append(args.tag)
    if is_integration:
        eval_tags.append("integration")

    eval_name = args.name or f"SWE-bench Lite ({len(instances)} cases)"

    target = {
        "type": "session",
        "harness_name": args.harness,
    }

    ev = client.create_eval(eval_name, target=target, tags=eval_tags)
    eval_id = ev["id"]
    print(f"Created eval: {eval_id} — {eval_name}")

    case_tags = []
    if is_integration:
        case_tags.append("integration")

    for i, instance in enumerate(instances):
        case = build_case(instance, tags=case_tags or None)
        result = client.create_case(eval_id, case)
        case_id = result.get("id", "?")
        print(f"  [{i + 1}/{len(instances)}] {instance['instance_id']} → {case_id}")

    print(f"\nDone. Eval {eval_id} has {len(instances)} cases.")
    print(f"Next: python -m swe_bench.runner --eval-id {eval_id}")

    if args.output:
        manifest = {
            "eval_id": eval_id,
            "instance_count": len(instances),
            "instance_ids": [inst["instance_id"] for inst in instances],
        }
        with open(args.output, "w") as f:
            json.dump(manifest, f, indent=2)
        print(f"Manifest written to {args.output}")

    client.close()


def main():
    parser = argparse.ArgumentParser(description="Load SWE-bench Lite into Everruns evals")
    parser.add_argument("--base-url", default=None, help="Everruns API base URL")
    parser.add_argument("--api-key", default=None, help="Everruns API key")
    parser.add_argument("--harness", default="coding-daytona", help="Harness name for sessions")
    parser.add_argument("--name", default=None, help="Eval name")
    parser.add_argument("--limit", type=int, default=None, help="Max instances to load")
    parser.add_argument("--integration", action="store_true", help="Load only 2 integration test instances")
    parser.add_argument("--tag", default=None, help="Extra tag for the eval")
    parser.add_argument("--output", "-o", default=None, help="Write manifest JSON to file")
    args = parser.parse_args()
    run(args)


if __name__ == "__main__":
    main()
