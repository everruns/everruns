#!/usr/bin/env python3
"""Reject empty/failed Rust analysis and missing workspace target roots.

Native semantic coverage is not evidence for wasm-only code. Retain the complete
CSV and SARIF diagnostics so warnings and files without semantics remain visible.
"""
import csv
import json
from pathlib import Path
import sys


def verify(metadata, rows, sarif):
    failures = []
    root = Path(metadata["workspace_root"])
    files = {row["path"]: row for row in rows}
    members = set(metadata["workspace_members"])
    targets = set()
    for package in metadata["packages"]:
        if package["id"] in members:
            for target in package["targets"]:
                targets.add(str(Path(target["src_path"]).relative_to(root)))
    if not targets:
        failures.append("Workspace has no targets")
    for target in sorted(targets):
        row = files.get(target, {})
        if row.get("semantics") != "true" or int(row.get("nodes", 0)) == 0:
            failures.append(f"Missing semantic AST for workspace target: {target}")
    runs = sarif.get("runs", [])
    if not runs:
        failures.append("SARIF has no runs")
    warnings = []
    for run in runs:
        driver = run.get("tool", {}).get("driver", {})
        if driver.get("name") != "CodeQL" or not any(
            rule.get("id", "").startswith("rust/") for rule in driver.get("rules", [])
        ):
            failures.append("Missing CodeQL Rust query rules")
        invocations = run.get("invocations", [])
        if not invocations:
            failures.append("Missing query execution evidence")
        for invocation in invocations:
            if invocation.get("executionSuccessful") is not True:
                failures.append("CodeQL execution was not successful")
            for key in ("toolExecutionNotifications", "toolConfigurationNotifications"):
                for diagnostic in invocation.get(key, []):
                    message = diagnostic.get("message", {}).get("text", str(diagnostic))
                    if diagnostic.get("level") == "error":
                        failures.append(message)
                    elif diagnostic.get("level") == "warning":
                        warnings.append(message)
    return targets, failures, warnings


def main():
    metadata_path, files_path, sarif_path = map(Path, sys.argv[1:])
    with files_path.open() as source:
        rows = list(csv.DictReader(source))
    targets, failures, warnings = verify(
        json.loads(metadata_path.read_text()), rows, json.loads(sarif_path.read_text())
    )
    print(f"Native extraction: {len(rows)} files; {len(targets)} workspace targets")
    print(f"Diagnostics: {len(failures)} errors; {len(warnings)} warnings")
    for message in warnings:
        print(f"WARNING: {message}")
    for message in failures:
        print(f"ERROR: {message}")
    print("Native coverage does not establish WASM-only coverage.")
    return bool(failures)


if __name__ == "__main__":
    sys.exit(main())
