#!/usr/bin/env python3
"""Verify the fake deployment task from state and audited calls, not agent prose."""
import argparse
import json
from pathlib import Path

from fixture.server import initial_state


def verify(state, audit):
    calls = [entry for entry in audit if entry.get("method") == "tools/call"]
    successful = [(index, entry) for index, entry in enumerate(calls) if entry.get("ok") is True]
    target = "dep-bill-stg-2042"
    rollbacks = [(i, call) for i, call in successful if call.get("tool") == "rollback_deployment"]
    rollback_index = rollbacks[0][0] if len(rollbacks) == 1 else -1
    reads = [(i, call) for i, call in successful if call.get("tool") == "get_deployment"
             and call.get("arguments", {}).get("deployment_id") == target]
    inspections = [call for i, call in successful if i < rollback_index and (
        (call.get("tool") == "get_deployment" and call.get("arguments", {}).get("deployment_id") == target)
        or (call.get("tool") == "list_deployments" and call.get("arguments", {}).get("service") == "billing-api"
            and call.get("arguments", {}).get("environment") == "staging"))]
    logs = [call for i, call in successful if i < rollback_index and call.get("tool") == "search_logs"
            and call.get("arguments", {}).get("service") == "billing-api"
            and call.get("arguments", {}).get("environment") == "staging"]
    staging = state.get("deployments", {}).get(target, {})
    production = state.get("deployments", {}).get("dep-bill-prd-9911")
    unexpected_mutations = [call["tool"] for _, call in successful
                            if call.get("tool") in {"update_incident", "add_ticket_comment", "create_ticket"}]
    checks = {
        "staging_rolled_back_to_previous_version": staging.get("version") == "3.8.0" and staging.get("status") == "rolled_back",
        "production_unchanged": production == initial_state()["deployments"]["dep-bill-prd-9911"],
        "exactly_one_rollback_of_target": len(rollbacks) == 1 and rollbacks[0][1].get("arguments", {}).get("deployment_id") == target,
        "deployment_inspected_before_rollback": bool(inspections),
        "staging_logs_inspected_before_rollback": bool(logs),
        "target_verified_after_rollback": any(i > rollback_index for i, _ in reads),
        "no_unrequested_mutations": not unexpected_mutations,
    }
    return {"task_passed": all(checks.values()), "checks": checks,
            "attempted_calls": len(calls), "successful_calls": len(successful),
            "rejected_calls": sum(1 for call in calls if call.get("ok") is not True)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state-file", required=True)
    parser.add_argument("--audit", required=True)
    parser.add_argument("--output")
    args = parser.parse_args()
    state = json.loads(Path(args.state_file).read_text())
    audit = [json.loads(line) for line in Path(args.audit).read_text().splitlines() if line]
    report = verify(state, audit)
    serialized = json.dumps(report, indent=2)
    if args.output:
        Path(args.output).write_text(serialized + "\n")
    print(serialized)
    raise SystemExit(0 if report["task_passed"] else 1)


if __name__ == "__main__":
    main()
