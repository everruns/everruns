#!/usr/bin/env python3
"""
HackerNews Reader Agent — Everruns Example

Equivalent to the GitHub Copilot SDK HackerNews demo
(https://x.com/i/status/2019435734365794710), but built on Everruns.

Instead of defining tool functions manually, the agent uses Everruns capabilities
(web_fetch, current_time, session_file_system) which provide tools automatically.
The Everruns platform handles orchestration, tool execution, and durable state.

Usage:
    # Start the Everruns server first:
    just start-dev

    # Run this example:
    python examples/hackernews-reader/run.py

    # With a custom prompt:
    python examples/hackernews-reader/run.py "Show me the top 3 Ask HN posts"

    # With a custom server URL:
    EVERRUNS_API_URL=http://localhost:9000 python examples/hackernews-reader/run.py
"""

import json
import os
import sys
import time

import requests

BASE_URL = os.environ.get("EVERRUNS_API_URL", "http://localhost:9000")
API = f"{BASE_URL}/v1"

# Read agent definition from the markdown file
AGENT_FILE = os.path.join(
    os.path.dirname(__file__), "..", "agents", "hackernews-reader.md"
)

DEFAULT_PROMPT = (
    "What are the top 5 stories on HackerNews right now? "
    "For the most popular one, show me the top comments and "
    "look up the author's profile."
)


def parse_agent_markdown(path: str) -> dict:
    """Parse agent markdown file with YAML frontmatter into API request."""
    with open(path) as f:
        content = f.read()

    if not content.startswith("---"):
        raise ValueError("Agent file must start with YAML frontmatter (---)")

    # Split frontmatter from body
    parts = content.split("---", 2)
    if len(parts) < 3:
        raise ValueError("Invalid frontmatter format")

    # Lazy import — only needed for parsing
    try:
        import yaml
    except ImportError:
        print("PyYAML required: pip install pyyaml")
        sys.exit(1)

    meta = yaml.safe_load(parts[1])
    system_prompt = parts[2].strip()

    request = {
        "name": meta["name"],
        "system_prompt": system_prompt,
    }
    if meta.get("description"):
        request["description"] = meta["description"]
    if meta.get("tags"):
        request["tags"] = meta["tags"]
    if meta.get("capabilities"):
        request["capabilities"] = [{"ref": c} for c in meta["capabilities"]]
    return request


def create_agent() -> str:
    """Create the HackerNews reader agent. Returns agent ID."""
    agent_req = parse_agent_markdown(AGENT_FILE)
    resp = requests.post(f"{API}/agents", json=agent_req)
    resp.raise_for_status()
    agent = resp.json()
    print(f"Agent created: {agent['id']} ({agent['name']})")
    return agent["id"]


def create_session(agent_id: str) -> str:
    """Create a session for the agent. Returns session ID."""
    resp = requests.post(
        f"{API}/sessions",
        json={"agent_id": agent_id, "title": "HackerNews Reader Session"},
    )
    resp.raise_for_status()
    session = resp.json()
    print(f"Session created: {session['id']}")
    return session["id"]


def send_message(session_id: str, text: str) -> str:
    """Send a user message. Returns message ID."""
    resp = requests.post(
        f"{API}/sessions/{session_id}/messages",
        json={
            "message": {
                "role": "user",
                "content": [{"type": "text", "text": text}],
            }
        },
    )
    resp.raise_for_status()
    msg = resp.json()
    print(f"\nUser: {text}\n")
    return msg["id"]


def stream_response(session_id: str, timeout: int = 120):
    """Poll events until turn completes. Print agent messages and tool calls."""
    url = f"{API}/sessions/{session_id}/events"
    seen = set()
    start = time.time()

    while time.time() - start < timeout:
        resp = requests.get(url)
        resp.raise_for_status()
        events = resp.json().get("data", [])

        for event in events:
            seq = event["sequence"]
            if seq in seen:
                continue
            seen.add(seq)

            etype = event["event_type"]
            data = event.get("data", {})

            if etype == "message.agent":
                # Agent's text response
                content = data.get("content", [])
                for part in content:
                    if part.get("type") == "text":
                        print(f"Agent: {part['text']}")

            elif etype == "message.tool_call":
                # Tool invocation — show what the agent is doing
                tool_name = data.get("name", "unknown")
                args = data.get("arguments", {})
                if tool_name == "web_fetch":
                    print(f"  -> Fetching: {args.get('url', '?')}")
                elif tool_name == "get_current_time":
                    print(f"  -> Checking current time")
                else:
                    print(f"  -> Tool: {tool_name}")

            elif etype == "message.tool_result":
                # Silently consumed by the agent loop
                pass

            elif etype == "turn.completed":
                print("\n[Turn completed]")
                return

            elif etype == "turn.failed":
                print(f"\n[Turn failed: {data.get('error', 'unknown')}]")
                return

        time.sleep(0.5)

    print("\n[Timed out waiting for response]")


def cleanup(agent_id: str, session_id: str):
    """Delete session and archive agent."""
    requests.delete(f"{API}/sessions/{session_id}")
    requests.delete(f"{API}/agents/{agent_id}")
    print("\nCleaned up agent and session.")


def main():
    prompt = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_PROMPT

    # Health check
    try:
        resp = requests.get(f"{BASE_URL}/health", timeout=5)
        resp.raise_for_status()
    except requests.ConnectionError:
        print(f"Cannot connect to Everruns at {BASE_URL}")
        print("Start the server first: just start-dev")
        sys.exit(1)

    agent_id = create_agent()
    session_id = create_session(agent_id)

    try:
        send_message(session_id, prompt)
        stream_response(session_id)

        # Interactive follow-up loop
        while True:
            try:
                follow_up = input("\nYou (or 'quit'): ").strip()
            except (EOFError, KeyboardInterrupt):
                break
            if not follow_up or follow_up.lower() in ("quit", "exit", "q"):
                break
            send_message(session_id, follow_up)
            stream_response(session_id)
    finally:
        cleanup(agent_id, session_id)


if __name__ == "__main__":
    main()
