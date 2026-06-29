"""Load the platform-capability dataset into an Everruns eval.

Reads dataset.yaml, creates one Eval, and adds one EvalCase per entry via the
/v1/evals API. The dataset is the source of truth; this script only translates
its runner-agnostic shape into the API's case payload.

Usage:
    python -m platform_capability.loader                 # full suite
    python -m platform_capability.loader --tag agents    # only cases tagged "agents"
    python -m platform_capability.loader -o manifest.json
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

import yaml

from .api import EvalClient

DATASET_PATH = Path(__file__).parent / "dataset.yaml"


def load_dataset(path: Path) -> dict:
    with open(path) as f:
        return yaml.safe_load(f)


def build_case_payload(case: dict, defaults: dict) -> dict:
    """Translate a dataset case into the /v1/evals/{id}/cases request body."""
    conversation = [{"content": msg} for msg in case["conversation"]]
    payload: dict[str, Any] = {
        "name": case["name"],
        "conversation": conversation,
        "scorers": case["scorers"],
        "max_turns": case.get("max_turns", defaults.get("max_turns", 10)),
        "timeout_seconds": case.get(
            "timeout_seconds", defaults.get("timeout_seconds", 120)
        ),
    }
    if case.get("description"):
        payload["description"] = case["description"]
    if case.get("tags"):
        payload["tags"] = case["tags"]
    return payload


def run(args: argparse.Namespace) -> None:
    data = load_dataset(Path(args.dataset))
    eval_spec = data["eval"]
    defaults = data.get("defaults", {})
    cases = data["cases"]

    if args.tag:
        cases = [c for c in cases if args.tag in (c.get("tags") or [])]
        if not cases:
            print(f"No cases matched tag '{args.tag}'", file=sys.stderr)
            sys.exit(1)

    target = dict(eval_spec.get("target") or {})
    if args.harness:
        target = {"type": "session", "harness_name": args.harness}

    client = EvalClient(base_url=args.base_url, api_key=args.api_key)

    eval_name = args.name or eval_spec["name"]
    ev = client.create_eval(
        eval_name,
        description=eval_spec.get("description"),
        target=target or None,
        tags=eval_spec.get("tags"),
    )
    eval_id = ev["id"]
    print(f"Created eval: {eval_id} — {eval_name}")

    for i, case in enumerate(cases):
        payload = build_case_payload(case, defaults)
        result = client.create_case(eval_id, payload)
        case_id = result.get("id", "?")
        print(f"  [{i + 1}/{len(cases)}] {case['name']} → {case_id}")

    print(f"\nDone. Eval {eval_id} has {len(cases)} cases.")
    print(f"Next: python -m platform_capability.runner --eval-id {eval_id}")

    if args.output:
        manifest = {
            "eval_id": eval_id,
            "case_count": len(cases),
            "case_names": [c["name"] for c in cases],
        }
        with open(args.output, "w") as f:
            json.dump(manifest, f, indent=2)
        print(f"Manifest written to {args.output}")

    client.close()


def main():
    parser = argparse.ArgumentParser(
        description="Load the platform-capability dataset into an Everruns eval"
    )
    parser.add_argument("--base-url", default=None, help="Everruns API base URL")
    parser.add_argument("--api-key", default=None, help="Everruns API key")
    parser.add_argument(
        "--dataset", default=str(DATASET_PATH), help="Path to dataset.yaml"
    )
    parser.add_argument(
        "--harness",
        default=None,
        help="Override target harness name (default: from dataset, platform-chat)",
    )
    parser.add_argument("--name", default=None, help="Override eval name")
    parser.add_argument("--tag", default=None, help="Only load cases with this tag")
    parser.add_argument("--output", "-o", default=None, help="Write manifest JSON")
    args = parser.parse_args()
    run(args)


if __name__ == "__main__":
    main()
