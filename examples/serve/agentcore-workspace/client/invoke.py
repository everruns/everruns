#!/usr/bin/env python3
"""Talk to the agentcore-workspace agent on AgentCore Runtime with IAM auth.

    pip install boto3
    python3 invoke.py --arn "$AGENT_ARN" "Add 'check the build' to my todo list and share it."

Each call is one turn. The session id is kept in `.agentcore-session`, so the
next call lands in the same microVM, conversation and workspace; pass
`--new` to start over. When the agent asks to share a report, the script asks
you (or `--approve` / `--reject` answers for you) and resumes the turn.

`--url http://127.0.0.1:8080/invocations` talks to a local
`cargo run -p serve-example-agentcore-workspace -- agentcore --dev` instead.
"""

import argparse
import json
import pathlib
import urllib.request
import uuid

SESSION_FILE = pathlib.Path(".agentcore-session")
SESSION_HEADER = "X-Amzn-Bedrock-AgentCore-Runtime-Session-Id"


def session_id(new: bool) -> str:
    if not new and SESSION_FILE.exists():
        return SESSION_FILE.read_text().strip()
    # AgentCore wants at least 33 characters.
    session = f"{uuid.uuid4()}-{uuid.uuid4()}"[:48]
    SESSION_FILE.write_text(session)
    return session


def sse_events(lines):
    """AG-UI events from an iterator of SSE lines (bytes)."""
    for raw in lines:
        line = raw.decode().rstrip("\r\n")
        if line.startswith("data:"):
            yield json.loads(line[5:].strip())


def invoke(args, session: str, body: dict):
    payload = json.dumps(body).encode()
    if args.url:
        request = urllib.request.Request(
            args.url,
            data=payload,
            headers={"content-type": "application/json", SESSION_HEADER: session},
        )
        with urllib.request.urlopen(request) as response:
            yield from sse_events(response)
        return
    import boto3

    client = boto3.client("bedrock-agentcore", region_name=args.region)
    response = client.invoke_agent_runtime(
        agentRuntimeArn=args.arn,
        runtimeSessionId=session,
        payload=payload,
        contentType="application/json",
        accept="text/event-stream",
    )
    yield from sse_events(response["response"].iter_lines(keepends=True))


def run(args, session: str, body: dict):
    """Print one run; return its open interrupts."""
    interrupts = []
    for event in invoke(args, session, body):
        kind = event.get("type")
        if kind == "TEXT_MESSAGE_CONTENT":
            print(event["delta"], end="", flush=True)
        elif kind == "TEXT_MESSAGE_END":
            print()
        elif kind == "TOOL_CALL_START":
            print(f"  [tool] {event.get('toolCallName')}")
        elif kind == "RUN_ERROR":
            print(f"  [error] {event.get('message')}")
        elif kind == "RUN_FINISHED":
            outcome = event.get("outcome") or {}
            if outcome.get("type") == "interrupt":
                interrupts = outcome.get("interrupts", [])
    return interrupts


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("prompt")
    parser.add_argument("--arn", help="the agent runtime ARN")
    parser.add_argument("--url", help="a local /invocations URL instead of AgentCore")
    parser.add_argument("--region", default="us-east-1")
    parser.add_argument("--new", action="store_true", help="start a new session")
    answer = parser.add_mutually_exclusive_group()
    answer.add_argument("--approve", action="store_true")
    answer.add_argument("--reject", action="store_true")
    args = parser.parse_args()
    if not (args.arn or args.url):
        parser.error("pass --arn (AgentCore) or --url (local)")

    session = session_id(args.new)
    print(f"session {session}")
    interrupts = run(args, session, {"prompt": args.prompt})
    while interrupts:
        resume = []
        for interrupt in interrupts:
            if args.approve or args.reject:
                decision = "allow" if args.approve else "reject"
            else:
                reply = input(f"  {interrupt.get('reason')}: {interrupt.get('message', '')} allow? [y/N] ")
                decision = "allow" if reply.strip().lower().startswith("y") else "reject"
            print(f"  -> {decision}")
            resume.append(
                {"interruptId": interrupt["id"], "status": "resolved", "payload": {"decision": decision}}
            )
        body = {
            "threadId": session,
            "runId": str(uuid.uuid4()),
            "messages": [],
            "tools": [],
            "context": [],
            "state": {},
            "forwardedProps": {},
            "resume": resume,
        }
        interrupts = run(args, session, body)


if __name__ == "__main__":
    main()
