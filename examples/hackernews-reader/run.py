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

import asyncio
import os
import sys

from everruns_sdk import Everruns

AGENT_FILE = os.path.join(os.path.dirname(__file__), "hackernews-reader.md")

DEFAULT_PROMPT = (
    "What are the top 5 stories on HackerNews right now? "
    "For the most popular one, show me the top comments and "
    "look up the author's profile."
)

# Dev mode needs no real key; use EVERRUNS_API_KEY env var in production
DEFAULT_API_KEY = "dev"
DEFAULT_BASE_URL = "http://localhost:9000"


async def main():
    prompt = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_PROMPT

    api_key = os.environ.get("EVERRUNS_API_KEY", DEFAULT_API_KEY)
    base_url = os.environ.get("EVERRUNS_API_URL", DEFAULT_BASE_URL)
    client = Everruns(api_key=api_key, base_url=base_url)

    # Import agent directly from markdown (name, capabilities, etc. parsed server-side)
    with open(AGENT_FILE) as f:
        agent = await client.agents.import_agent(f.read())
    print(f"Agent created: {agent.id} ({agent.name})")

    # Create session
    session = await client.sessions.create(
        agent_id=agent.id,
        title="HackerNews Reader Session",
    )
    print(f"Session created: {session.id}")

    try:
        # Send initial message
        await client.messages.create(session.id, prompt)
        print(f"\nUser: {prompt}\n")

        # Stream events until turn completes
        await print_events(client, session.id)

        # Interactive follow-up loop
        while True:
            try:
                follow_up = input("\nYou (or 'quit'): ").strip()
            except (EOFError, KeyboardInterrupt):
                break
            if not follow_up or follow_up.lower() in ("quit", "exit", "q"):
                break

            await client.messages.create(session.id, follow_up)
            print(f"\nUser: {follow_up}\n")
            await print_events(client, session.id)
    finally:
        await client.sessions.delete(session.id)
        await client.agents.delete(agent.id)
        await client.close()
        print("\nCleaned up agent and session.")


async def print_events(client: Everruns, session_id: str):
    """Stream events and print agent responses / tool calls."""
    async for event in client.events.stream(session_id):
        if event.type == "output.message.completed":
            # Extract text from message content parts
            message = event.data.get("message", {})
            for part in message.get("content", []):
                if part.get("type") == "text":
                    print(f"Agent: {part['text']}")

        elif event.type == "tool.started":
            tool_call = event.data.get("tool_call", {})
            tool_name = tool_call.get("name", "unknown")
            args = tool_call.get("arguments", {})
            if tool_name == "web_fetch":
                print(f"  -> Fetching: {args.get('url', '?')}")
            elif tool_name == "get_current_time":
                print("  -> Checking current time")
            else:
                print(f"  -> Tool: {tool_name}")

        elif event.type == "turn.completed":
            print("\n[Turn completed]")
            return

        elif event.type == "turn.failed":
            print(f"\n[Turn failed: {event.data.get('error', 'unknown')}]")
            return


if __name__ == "__main__":
    asyncio.run(main())
