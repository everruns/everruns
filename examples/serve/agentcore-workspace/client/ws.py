#!/usr/bin/env python3
"""Talk to the agentcore-workspace agent over AgentCore's WebSocket transport.

    pip install boto3 websockets
    python3 ws.py --arn "$AGENT_ARN" "Add 'check the build' to my todo list and share it." "What is on it now?"

Every prompt is one turn on the same socket. The upgrade request is signed with
SigV4 (service `bedrock-agentcore`) from the default AWS credential chain; boto3
has no WebSocket client, so the script signs the handshake itself and hands the
headers to `websockets`. `--presign` puts the signature in the URL instead,
which is what a browser, which cannot set upgrade headers, would use.

The session id is shared with `invoke.py` (`.agentcore-session`), so both
clients land in the same microVM, conversation and workspace. `--url
ws://127.0.0.1:8080/ws` talks to a local `agentcore --dev` instead.
"""

import argparse
import asyncio
import json
import pathlib
import urllib.parse
import uuid

import websockets

SESSION_FILE = pathlib.Path(".agentcore-session")
SESSION_HEADER = "X-Amzn-Bedrock-AgentCore-Runtime-Session-Id"


def session_id(new: bool) -> str:
    if not new and SESSION_FILE.exists():
        return SESSION_FILE.read_text().strip()
    # AgentCore wants at least 33 characters.
    session = f"{uuid.uuid4()}-{uuid.uuid4()}"[:48]
    SESSION_FILE.write_text(session)
    return session


def signed(arn: str, region: str, session: str, presign: bool):
    """The wss:// URL and headers for a SigV4-signed upgrade."""
    import boto3
    from botocore.auth import SigV4Auth, SigV4QueryAuth
    from botocore.awsrequest import AWSRequest

    credentials = boto3.Session().get_credentials().get_frozen_credentials()
    host = f"bedrock-agentcore.{region}.amazonaws.com"
    base = f"https://{host}/runtimes/{urllib.parse.quote(arn, safe='')}/ws"
    if presign:
        query = urllib.parse.urlencode({"qualifier": "DEFAULT", SESSION_HEADER: session})
        request = AWSRequest("GET", f"{base}?{query}")
        SigV4QueryAuth(credentials, "bedrock-agentcore", region, expires=300).add_auth(request)
        headers = {}
    else:
        request = AWSRequest("GET", f"{base}?qualifier=DEFAULT", headers={SESSION_HEADER: session, "host": host})
        SigV4Auth(credentials, "bedrock-agentcore", region).add_auth(request)
        headers = {k: v for k, v in request.headers.items() if k.lower() != "host"}
    return request.url.replace("https://", "wss://", 1), headers


async def run(ws, body: dict):
    """Send one invocation body, print its events; return its open interrupts."""
    await ws.send(json.dumps(body))
    while True:
        event = json.loads(await ws.recv())
        kind = event.get("type")
        if kind == "TEXT_MESSAGE_CONTENT":
            print(event["delta"], end="", flush=True)
        elif kind == "TEXT_MESSAGE_END":
            print()
        elif kind == "TOOL_CALL_START":
            print(f"  [tool] {event.get('toolCallName')}")
        elif kind == "RUN_ERROR":
            print(f"  [error] {event.get('message')}")
            return []
        elif kind == "RUN_FINISHED":
            outcome = event.get("outcome") or {}
            return outcome.get("interrupts", []) if outcome.get("type") == "interrupt" else []


def answer(args, interrupt) -> str:
    if args.approve or args.reject:
        return "allow" if args.approve else "reject"
    reply = input(f"  {interrupt.get('reason')}: {interrupt.get('message', '')} allow? [y/N] ")
    return "allow" if reply.strip().lower().startswith("y") else "reject"


async def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("prompts", nargs="+", help="one turn each, on the same socket")
    parser.add_argument("--arn", help="the agent runtime ARN")
    parser.add_argument("--url", help="a local ws:// URL instead of AgentCore")
    parser.add_argument("--region", default="us-east-1")
    parser.add_argument("--presign", action="store_true", help="sign the URL instead of the headers")
    parser.add_argument("--new", action="store_true", help="start a new session")
    choice = parser.add_mutually_exclusive_group()
    choice.add_argument("--approve", action="store_true")
    choice.add_argument("--reject", action="store_true")
    args = parser.parse_args()
    if not (args.arn or args.url):
        parser.error("pass --arn (AgentCore) or --url (local)")

    session = session_id(args.new)
    if args.url:
        url, headers = args.url, {SESSION_HEADER: session}
    else:
        url, headers = signed(args.arn, args.region, session, args.presign)
    print(f"session {session}")
    async with websockets.connect(url, additional_headers=headers, max_size=None) as ws:
        for prompt in args.prompts:
            print(f"> {prompt}")
            interrupts = await run(ws, {"prompt": prompt})
            while interrupts:
                resume = []
                for interrupt in interrupts:
                    decision = answer(args, interrupt)
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
                interrupts = await run(ws, body)


if __name__ == "__main__":
    asyncio.run(main())
