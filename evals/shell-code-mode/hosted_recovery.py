#!/usr/bin/env python3
"""Qualify a provisioned Monty host across an operator-controlled process fault.

Dedicated synthetic agents need python_code_mode, session_file_system,
session_sql_database and tool_approval (normal, rule delete_file=ask). The fault
hook is trusted operator argv, never model output. It must replace the executor
or host and return JSON evidence after the endpoint is ready again.
"""
import argparse
import json
import os
from pathlib import Path
import shlex
import subprocess
import time
import urllib.parse
import uuid

from hosted_smoke import payload, request


def objects(value):
    if isinstance(value, dict):
        yield value
        for child in value.values():
            yield from objects(child)
    elif isinstance(value, list):
        for child in value:
            yield from objects(child)
    elif isinstance(value, str):
        try:
            child = json.loads(value)
        except ValueError:
            return
        if not isinstance(child, str):
            yield from objects(child)


def events_at(base):
    events, cursor = [], None
    while True:
        query = {"limit": 1000}
        if cursor:
            query["since_id"] = cursor
        batch = payload(request("GET", base + "/events?" + urllib.parse.urlencode(query)))
        if not isinstance(batch, list):
            raise ValueError("expected event array")
        events.extend(batch)
        if len(events) > 10000:
            raise ValueError("recovery trace exceeds the fixture's event limit")
        if len(batch) < 1000:
            return events
        cursor = batch[-1]["id"]


def await_events(base, predicate, timeout):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        events = events_at(base)
        if predicate(events):
            return events
        if any(e["type"] == "turn.failed" for e in events):
            raise RuntimeError("hosted turn failed")
        time.sleep(0.25)
    raise TimeoutError("hosted recovery wait expired")


def approval(events):
    calls = [c for e in events if e["type"] == "tool.call_requested"
             for c in e["data"].get("tool_calls", [])]
    if not calls:
        return None
    if len(calls) != 1 or calls[0]["name"] != "approve_tool_call":
        raise ValueError("unexpected approval requests")
    call = calls[0]
    if call["arguments"]["tool"] != "delete_file" or call["arguments"]["arguments"] != {"path": "/workspace/monty-proof.txt"}:
        raise ValueError("refusing to answer an undeclared action")
    return call


def awaiting_approval(events):
    pending = approval(events)
    if pending is None and any(e["type"] == "turn.completed" for e in events):
        raise RuntimeError("program ended without requesting the declared approval")
    return pending is not None


def exists(base):
    try:
        request("GET", base + "/fs/monty-proof.txt")
        return True
    except RuntimeError as error:
        if str(error) == "API returned HTTP 404":
            return False
        raise


def send(base, placement, prompt):
    body = {"text": prompt} if placement == "agentd" else {"message": {"role": "user", "content": [{"type": "text", "text": prompt}]}}
    return request("POST", base + "/messages", body)


def grade(events, decision, file_exists, fault, expected_script=None):
    starts = [e["data"]["tool_call"] for e in events if e["type"] == "tool.started"]
    python = [c for c in starts if c["name"] == "python"]
    parents = {c["id"] for c in python}
    nested = [e["data"] for e in events if e["type"] == "tool.nested_call" and e["data"].get("status") == "completed" and e["data"].get("parent_tool_call_id") in parents]
    queries = {c["id"] for c in starts if c["name"] == "sql_query"
               and c["arguments"].get("database") == "montyproof"
               and c["arguments"].get("sql", "").strip().rstrip(";").strip().casefold() == "select count(*) as receipts from receipts"}
    receipt = any(o.get("columns") == ["receipts"] and o.get("rows") == [[1]]
                  for e in events if e["type"] == "tool.completed" and e["data"].get("tool_call_id") in queries and e["data"].get("tool_name") == "sql_query" and e["data"].get("success")
                  for o in objects(e["data"].get("result")))
    old_pid, new_pid = fault.get("old_executor_pid"), fault.get("new_executor_pid")
    checks = {
        "one_program": sum("script" in c["arguments"] for c in python) == 1,
        "saved_resume": sum("resume" in c["arguments"] for c in python) == 1,
        "one_receipt_effect": sum(c["tool_name"] == "sql_execute" for c in nested) == 1 and receipt,
        "expected_delete": sum(c["tool_name"] == "delete_file" for c in nested) == int(decision == "allow"),
        "no_direct_effect_fallback": not any(c["name"] in ("sql_execute", "delete_file") for c in starts),
        "file_state": file_exists == (decision == "reject"),
        "fault_observed": type(old_pid) is int and type(new_pid) is int and old_pid > 0 and new_pid > 0 and old_pid != new_pid and fault.get("old_executor_exited") is True,
        "no_turn_failure": not any(e["type"] == "turn.failed" for e in events),
    }
    if expected_script is not None:
        checks["exact_program"] = [c["arguments"]["script"] for c in python if "script" in c["arguments"]] == [expected_script]
    if "leader_before" in fault:
        checks["leader_changed"] = fault["leader_before"] != fault.get("leader_after")
    return {"passed": all(checks.values()), "checks": checks}


def run(args):
    parsed = urllib.parse.urlsplit(args.sessions_url)
    if parsed.username or parsed.password or parsed.query or parsed.fragment or parsed.scheme not in ("http", "https"):
        raise ValueError("invalid sessions URL")
    if parsed.scheme != "https" and parsed.hostname not in ("localhost", "127.0.0.1", "::1"):
        raise ValueError("require HTTPS except on loopback")
    session = payload(request("POST", args.sessions_url, {"agent_id": args.agent}))["id"]
    args.session_id = session
    base = args.sessions_url.rstrip("/") + "/" + urllib.parse.quote(session, safe="")
    args.base = base
    request("POST", base + "/fs/monty-proof.txt", {"content": "synthetic recovery fixture"})
    marker = uuid.uuid4().hex
    script = "\n".join([
        "r = tool('sql_execute', {'database': 'montyproof', 'sql': \"CREATE TABLE receipts (marker TEXT); INSERT INTO receipts VALUES ('" + marker + "');\"})",
        "if r.get('error'): raise ValueError(r['error'])",
        "tool('delete_file', {'path': '/workspace/monty-proof.txt'})",
        "tool('stat_file', {'path': '/workspace/monty-proof.txt'})",
    ])
    send(base, args.placement, "Run this exact program once using python. It records a receipt before requesting deletion approval. After either approval or rejection, call python with the returned resume token exactly once, never rerun source or invoke the effect directly. Then stop.\n```python\n" + script + "\n```")
    before = await_events(base, awaiting_approval, args.timeout)
    pending = approval(before)
    if not exists(base):
        raise AssertionError("delete happened before approval")
    hook_env = dict(os.environ, MONTY_RECOVERY_SESSION_ID=session)
    fault = subprocess.run(shlex.split(args.fault_command), env=hook_env, check=True, capture_output=True, text=True, timeout=args.timeout)
    evidence = json.loads(fault.stdout)
    # Re-read the original pending request from durable events after replacement.
    after_fault = events_at(base)
    if approval(after_fault)["id"] != pending["id"] or not exists(base):
        raise AssertionError("pending approval or original file changed across the fault")
    request("POST", base + "/tool-approvals", {"decisions": [{"tool_call_id": pending["id"], "decision": args.decision}]})
    await_events(base, lambda e: any(x["type"] == "turn.completed" for x in e), args.timeout)
    send(base, args.placement, "Verification only: call sql_query directly with database montyproof and SQL SELECT COUNT(*) AS receipts FROM receipts. Do not write or call python. Report its result.")
    events = await_events(base, lambda e: sum(x["type"] == "turn.completed" for x in e) == 2, args.timeout)
    return {"placement": args.placement, "decision": args.decision, "session_id": session,
            "runtime_qualification": args.runtime_qualification, "fault": evidence,
            **grade(events, args.decision, exists(base), evidence, script), "events": events}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--placement", choices=("server", "agentd"), required=True)
    p.add_argument("--sessions-url", required=True)
    p.add_argument("--agent", required=True)
    p.add_argument("--decision", choices=("allow", "reject"), required=True)
    p.add_argument("--fault-command", required=True)
    p.add_argument("--runtime-qualification", required=True)
    p.add_argument("--timeout", type=int, default=180)
    p.add_argument("--output", type=Path, required=True)
    a = p.parse_args()
    try:
        report = run(a)
    except Exception as error:
        report = {"passed": False, "placement": a.placement, "decision": a.decision, "error": str(error), "session_id": getattr(a, "session_id", None)}
        if hasattr(a, "base"):
            try:
                report["events"] = events_at(a.base)
                request("POST", a.base + "/cancel", {})
            except Exception:
                pass
    a.output.parent.mkdir(parents=True, exist_ok=True)
    a.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({k: v for k, v in report.items() if k != "events"}))
    raise SystemExit(0 if report["passed"] else 1)


if __name__ == "__main__":
    main()
