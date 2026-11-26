#!/usr/bin/env python3
"""Boundary smoke for dedicated Server/agentd eval agents, never a model benchmark.

Creates a fresh synthetic session; preserves its id/events for inspection. Agent
configuration and host runtime provisioning stay with the operator. No secret
values, agent definitions or unrelated sessions are downloaded into reports.
"""
import argparse
import json
import os
import re
from pathlib import Path
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *_args, **_kwargs):
        raise RuntimeError("API redirect refused; use the canonical sessions URL")


def request(method, url, body=None):
    headers = {"Content-Type": "application/json"}
    token = os.environ.get("EVERRUNS_EVAL_API_KEY")
    if token:
        headers["Authorization"] = f"Bearer {token}"
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.build_opener(NoRedirect()).open(req, timeout=30) as response:
            return json.load(response)
    except urllib.error.HTTPError as error:
        raise RuntimeError(f"API returned HTTP {error.code}") from None


def payload(value):
    return value.get("data", value) if isinstance(value, dict) else value


def contains_artifact(value, expected):
    if value == expected:
        return True
    if isinstance(value, str):
        try:
            decoded = json.loads(value)
        except (ValueError, TypeError):
            return False
        return not isinstance(decoded, str) and contains_artifact(decoded, expected)
    if isinstance(value, dict):
        if isinstance(value.get("content"), str) and "lines_shown" in value:
            # Session read_file presents numbered lines; strip only that documented
            # envelope before parsing the complete JSON, never search arbitrary prose.
            raw = "\n".join(re.sub(r"^\d+\|", "", line) for line in value["content"].splitlines())
            if not value.get("truncated") and contains_artifact(raw, expected):
                return True
        return any(contains_artifact(v, expected) for v in value.values())
    if isinstance(value, list):
        return any(contains_artifact(v, expected) for v in value)
    return False


def grade(events, surface, marker):
    expected = {"sum": 650, "count": 12, "marker": marker}
    wrapper = {"monty": "python", "cli": "bash"}.get(surface)
    starts = {e["data"]["tool_call"]["id"]: e["data"]["tool_call"]["name"]
              for e in events if e.get("type") == "tool.started"}
    operations = []
    direct_fallback = False
    for event in events:
        data = event.get("data", {})
        if event.get("type") == "tool.started" and data["tool_call"]["name"] == "stat_file":
            direct_fallback |= wrapper is not None
        if event.get("type") == "tool.nested_call" and data.get("status") == "completed":
            if wrapper and starts.get(data.get("parent_tool_call_id")) == wrapper:
                operations.append(data.get("tool_name"))
        if event.get("type") == "tool.completed" and data.get("success"):
            name = data.get("tool_name")
            if name in ("write_file", "read_file") or wrapper is None:
                operations.append(name)
    # File hot paths deliberately stay direct. Verify their authoritative result;
    # stat_file must cross the selected adapter and report existing workspace state.
    artifact = any(e.get("type") == "tool.completed" and e.get("data", {}).get("success")
                   and e["data"].get("tool_name") == "read_file"
                   and contains_artifact(e["data"].get("result"), expected) for e in events)
    checks = {"turn_completed": any(e.get("type") == "turn.completed" for e in events),
              "one_write": operations.count("write_file") == 1,
              "read_back": operations.count("read_file") >= 1,
              "nested_stat": operations.count("stat_file") >= 1,
              "artifact_in_tool_result": artifact, "no_direct_fallback": not direct_fallback,
              "no_turn_failure": not any(e.get("type") == "turn.failed" for e in events)}
    return {"passed": all(checks.values()), "checks": checks}


def run(args):
    parsed = urllib.parse.urlsplit(args.sessions_url)
    if parsed.username or parsed.password or parsed.query or parsed.fragment:
        raise ValueError("Use a sessions collection URL without credentials, query or fragment")
    if parsed.scheme != "https" and not (parsed.scheme == "http" and parsed.hostname in ("localhost", "127.0.0.1", "::1")):
        raise ValueError("Require HTTPS, except for a local test host")
    marker = f"code-mode-eval-{uuid.uuid4().hex}"
    created = payload(request("POST", args.sessions_url, {"agent_id": args.agent}))
    session_id = created["id"]
    base = args.sessions_url.rstrip("/") + "/" + urllib.parse.quote(session_id, safe="")
    interface = {"native": "direct native tool calls", "cli": "the tools command inside bash", "monty": "tool(name, arguments) inside python"}[args.surface]
    prompt = (f"Use {interface} to compute the sum of squares of integers 1 through 12. "
              "The file hot paths write_file and read_file must be called directly, outside any execution wrapper. "
              f"Write a JSON object with keys sum, count, marker to /workspace/eval-result.json exactly once; "
              f"count is 12 and marker is {marker!r}. Read the file back with read_file. "
              f"Then invoke stat_file through {interface} to verify the file exists. "
              "Discover schemas as needed. Do not use shell filesystem commands or claim success without reading it.")
    body = {"text": prompt} if args.placement == "agentd" else {"message": {"role": "user", "content": [{"type": "text", "text": prompt}]}}
    request("POST", base + "/messages", body)
    events, seen, cursor = [], set(), None
    deadline = time.monotonic() + args.timeout
    while time.monotonic() < deadline:
        query = {"limit": 1000}
        if cursor:
            query["since_id"] = cursor
        batch = payload(request("GET", base + "/events?" + urllib.parse.urlencode(query)))
        if not isinstance(batch, list):
            raise ValueError("Events endpoint did not return a data array")
        for event in batch:
            if event["id"] not in seen:
                seen.add(event["id"])
                events.append(event)
            cursor = event["id"]
        if any(e.get("type") in ("turn.completed", "turn.failed", "tool.call_requested") for e in events):
            break
        if len(batch) < 1000:
            time.sleep(0.25)
    report = {"placement": args.placement, "surface": args.surface, "session_id": session_id,
              "scope": "hosted workspace boundary smoke; no restart/approval qualification or comparative performance claim",
              "runtime_qualification": args.runtime_qualification, **grade(events, args.surface, marker), "events": events}
    if not any(e.get("type") in ("turn.completed", "turn.failed") for e in events):
        request("POST", base + "/cancel", {})
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--placement", choices=["server", "agentd"], required=True)
    parser.add_argument("--surface", choices=["native", "cli", "monty"], required=True)
    parser.add_argument("--sessions-url", required=True)
    parser.add_argument("--agent", required=True, help="Dedicated eval agent with file tools and the selected execution capability")
    parser.add_argument("--runtime-qualification", required=True, help="Evidence reference or explicit unqualified/interpreter-only label")
    parser.add_argument("--timeout", type=int, default=120)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        report = run(args)
    except Exception as error:
        report = {"passed": False, "placement": args.placement, "surface": args.surface, "error_kind": "infrastructure", "error": str(error)}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({k: v for k, v in report.items() if k != "events"}))
    raise SystemExit(0 if report["passed"] else 1)


if __name__ == "__main__":
    main()
