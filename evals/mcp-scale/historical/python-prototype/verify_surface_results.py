"""Verify ticket-result parity from the real synthetic hybrid run report."""

import argparse
import json
from pathlib import Path


def result_object(call):
    return json.loads(call["result"][0]["text"])


def verify(report):
    calls = report["tool_calls"]
    nested = next(
        c for c in calls
        if c.get("command") == "tools ops catalog-call"
        and c.get("status") == "completed"
    )
    arguments = json.loads(nested["input_preview"])
    native = next(
        c for c in calls
        if c.get("tool_name") == "mcp_ops__catalog_call"
        and c.get("invocation_surface") == "native"
    )
    shell = next(
        c for c in calls
        if c.get("tool_name") == "bash"
        and c.get("tool_call_id") == nested["parent_tool_call_id"]
    )
    shell_result = result_object(shell)
    native_ticket = json.loads(result_object(native)["result"])
    cli_ticket = json.loads(json.loads(shell_result["stdout"])["result"])
    checks = {
        "runtime_completed": report.get("success") is True,
        "native_call_succeeded": native.get("success") is True,
        "shell_call_succeeded": shell.get("success") is True
        and shell_result.get("success") is True
        and shell_result.get("exit_code") == 0,
        "shell_result_untruncated": shell_result.get("truncated") is False,
        "expected_nested_operation": arguments.get("tool_id") == "platform::get_ticket"
        and arguments.get("arguments") == {"ticket_id": "OPS-8421"},
        "complete_ticket_results_equal": native_ticket == cli_ticket,
        "matches_fixture_ticket": native_ticket.get("ticket_id") == "OPS-8421"
        and native_ticket.get("owner") == "Ava Li"
        and native_ticket.get("status") == "in_progress",
    }
    return {
        "task_passed": all(checks.values()),
        "checks": checks,
        "native_ticket": native_ticket,
        "cli_ticket": cli_ticket,
        "scope": "Result parity for the synthetic gateway adapters; not a cache or retrieval benchmark.",
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    try:
        result = verify(json.loads(args.report.read_text()))
    except (KeyError, ValueError, TypeError, IndexError, StopIteration) as exc:
        result = {"task_passed": False, "error": f"Missing or invalid result evidence: {exc}"}
    output = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(output)
    else:
        print(output, end="")
    raise SystemExit(0 if result["task_passed"] else 1)


if __name__ == "__main__":
    main()
