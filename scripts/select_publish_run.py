#!/usr/bin/env python3
"""Select the uniquely correlated Publish Crate workflow run."""

from __future__ import annotations

import argparse
import json
import sys


def expected_title(package: str, tag: str, sha: str, correlation: str) -> str:
    return f"Publish {package} {tag} @ {sha} [{correlation}]"


def select_run(
    runs: list[dict], package: str, tag: str, sha: str, correlation: str
) -> int | None:
    """Return the unique intended run ID, ignoring unrelated concurrent runs."""
    if not isinstance(runs, list) or not all(isinstance(run, dict) for run in runs):
        raise ValueError("Publish Crate run listing must be a JSON array of objects")
    title = expected_title(package, tag, sha, correlation)
    matches = [
        run
        for run in runs
        if run.get("event") == "workflow_dispatch"
        and run.get("displayTitle") == title
        and run.get("headSha") == sha
        and isinstance(run.get("databaseId"), int)
    ]
    if len(matches) > 1:
        raise ValueError(f"multiple Publish Crate runs matched {title}")
    return matches[0]["databaseId"] if matches else None


def self_test() -> int:
    package = "everruns-core"
    tag = "crate/everruns-core/v0.24.0"
    sha = "a" * 40
    correlation = f"100-1-4-{sha}"
    intended = {
        "databaseId": 42,
        "displayTitle": expected_title(package, tag, sha, correlation),
        "event": "workflow_dispatch",
        "headSha": sha,
    }
    runs = [
        {
            "databaseId": 99,
            "displayTitle": expected_title(
                "everruns-host",
                "crate/everruns-host/v0.23.0",
                sha,
                f"manual-{sha}",
            ),
            "event": "workflow_dispatch",
            "headSha": sha,
        },
        {
            **intended,
            "databaseId": 77,
            "displayTitle": expected_title(package, tag, sha, f"stale-{sha}"),
        },
        intended,
    ]
    if select_run(runs, package, tag, sha, correlation) != 42:
        print("FAIL: concurrent unrelated dispatch displaced intended run", file=sys.stderr)
        return 1
    if select_run(runs, package, tag, "b" * 40, correlation) is not None:
        print("FAIL: wrong release SHA matched intended run", file=sys.stderr)
        return 1
    try:
        select_run([intended, intended], package, tag, sha, correlation)
    except ValueError:
        pass
    else:
        print("FAIL: duplicate correlated runs were accepted", file=sys.stderr)
        return 1
    print("publish-run selector self-test passed")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package")
    parser.add_argument("--tag")
    parser.add_argument("--sha")
    parser.add_argument("--correlation")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        return self_test()
    if not all((args.package, args.tag, args.sha, args.correlation)):
        parser.error("--package, --tag, --sha, and --correlation are required")
    try:
        run_id = select_run(
            json.load(sys.stdin),
            args.package,
            args.tag,
            args.sha,
            args.correlation,
        )
    except (json.JSONDecodeError, ValueError) as exc:
        print(f"::error::{exc}", file=sys.stderr)
        return 2
    if run_id is None:
        return 1
    print(run_id)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
