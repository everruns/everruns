#!/usr/bin/env python3
"""
Desktop X review: an Everruns agent opens a browser on a Sandbox desktop and
reviews the recent posts of an X (Twitter) account.

The agent gets one Daytona (or E2B) Sandbox with `desktop` turned on in its
Sandbox Template, so its shell, its files and its screen are the same machine. It
drives the screen through the `computer` tool and writes its review to a file
in the Sandbox workspace. The model is real (`gpt-6.1-sol`); every step is the
model looking at a screenshot and choosing the next action.

Usage:
    # Start Everruns (see AGENTS.md "Local dev"), then:
    DAYTONA_API_KEY=... python3 examples/desktop-x-review/run.py

    # Another account, provider, server or model:
    python3 examples/desktop-x-review/run.py --handle rustlang
    E2B_API_KEY=... python3 examples/desktop-x-review/run.py --provider e2b
    EVERRUNS_API_URL=http://localhost:9300/api EVERRUNS_MODEL=gpt-6-luna \\
        python3 examples/desktop-x-review/run.py

Only the Python standard library is needed.
"""

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request

AGENT_FILE = os.path.join(os.path.dirname(__file__), "desktop-x-review.md")
# The local dev proxy (PORT_PREFIX=271) serves the API under /api.
DEFAULT_BASE_URL = "http://localhost:27100/api"
DEFAULT_MODEL = "gpt-6.1-sol"
REVIEW_PATH = "x-review.md"
# A browser session on a desktop takes dozens of screenshot round trips.
TURN_TIMEOUT_SECONDS = 30 * 60


class Api:
    def __init__(self, base_url, api_key):
        self.base_url = base_url.rstrip("/")
        self.api_key = api_key

    def call(self, method, path, body=None):
        data = None if body is None else json.dumps(body).encode()
        request = urllib.request.Request(
            self.base_url + path, data=data, method=method
        )
        request.add_header("Content-Type", "application/json")
        if self.api_key:
            request.add_header("Authorization", f"Bearer {self.api_key}")
        try:
            with urllib.request.urlopen(request, timeout=120) as response:
                raw = response.read()
        except urllib.error.HTTPError as error:
            detail = error.read().decode(errors="replace")
            raise SystemExit(f"{method} {path} failed: {error.code} {detail}")
        return json.loads(raw) if raw else None


def read_agent_file():
    """Split the agent markdown into its front matter fields and prompt."""
    text = open(AGENT_FILE).read()
    _, front, prompt = text.split("---", 2)
    fields = {}
    for line in front.strip().splitlines():
        key, _, value = line.partition(":")
        fields[key.strip()] = value.strip().strip('"')
    return fields, prompt.strip()


def provider_connection(api, org, provider):
    """Reuse the example's provider account, or save the key as a new one."""
    name = f"Desktop X review ({provider})"
    listing = api.call("GET", f"/v1/orgs/{org}/organization-connections")
    rows = listing.get("data", listing) if isinstance(listing, dict) else listing
    for row in rows:
        if row.get("provider") == provider and row.get("name") == name:
            return row["id"]
    key_env = f"{provider.upper()}_API_KEY"
    api_key = os.environ.get(key_env)
    if not api_key:
        raise SystemExit(f"Set {key_env} to save a {provider} account.")
    created = api.call(
        "POST",
        f"/v1/orgs/{org}/organization-connections/providers/{provider}",
        {"name": name, "api_key": api_key},
    )
    return created["id"]


def sandbox_policy(provider, connection_id):
    """One fixed Sandbox with a desktop the agent can drive."""
    return {
        "mode": "fixed",
        "default": "desktop",
        "templates": {
            "desktop": {
                "target": {
                    "kind": "managed",
                    "provider": provider,
                    "credential": {
                        "source": "organization",
                        "connection_id": connection_id,
                    },
                    "options": {
                        "desktop": {
                            "display_width": 1280,
                            "display_height": 800,
                            "max_actions_per_session": 200,
                        }
                    },
                },
                # Physical sandboxes are disposable: delete this one as soon
                # as it goes idle, the session record stays.
                "lifecycle": {"idle_after_seconds": 600, "idle_action": "delete"},
            }
        },
    }


def describe(event):
    """One line for the events worth showing while the agent works."""
    kind = event.get("type", "")
    data = event.get("data") or {}
    if kind == "tool.started":
        call = data.get("tool_call") or {}
        args = call.get("arguments") or {}
        if call.get("name") == "computer":
            steps = []
            for action in args.get("actions", []):
                step = action.get("action", "")
                if action.get("text"):
                    step += f" {action['text']!r}"
                if action.get("coordinate"):
                    step += f" at {action['coordinate']}"
                steps.append(step)
            return "  [computer] " + ", ".join(steps)
        detail = args.get("command") or args.get("path") or json.dumps(args)
        return f"  [{call.get('name')}] {detail[:120]}"
    if kind == "tool.completed" and data.get("success") is False:
        return f"  tool failed: {json.dumps(data.get('error') or data)[:200]}"
    if kind in ("turn.failed", "turn.cancelled"):
        return f"  {kind}: {json.dumps(data)[:300]}"
    return None


def wait_for_turn(api, session_id):
    """Print progress until the turn ends; return the final assistant text."""
    since = None
    deadline = time.time() + TURN_TIMEOUT_SECONDS
    while time.time() < deadline:
        query = f"?since_id={since}" if since else ""
        page = api.call("GET", f"/v1/sessions/{session_id}/events{query}")
        for event in page.get("data", []):
            since = event["id"]
            line = describe(event)
            if line:
                print(line, flush=True)
            if event.get("type") == "turn.completed":
                return
            if event.get("type") in ("turn.failed", "turn.cancelled"):
                raise SystemExit("The turn did not complete.")
        time.sleep(3)
    raise SystemExit("Timed out waiting for the agent.")


def last_assistant_text(api, session_id):
    messages = api.call("GET", f"/v1/sessions/{session_id}/messages")
    for message in reversed(messages.get("data", [])):
        if message.get("role") == "agent" or message.get("role") == "assistant":
            parts = message.get("content") or []
            return "\n".join(p.get("text", "") for p in parts if p.get("text"))
    return ""


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--handle", default="everrunshq", help="X account to review")
    parser.add_argument(
        "--provider",
        choices=["daytona", "e2b"],
        default="daytona",
        help="Sandbox provider; its key comes from DAYTONA_API_KEY or E2B_API_KEY",
    )
    args = parser.parse_args()

    api = Api(
        os.environ.get("EVERRUNS_API_URL", DEFAULT_BASE_URL),
        os.environ.get("EVERRUNS_API_KEY", ""),
    )
    org = os.environ.get("EVERRUNS_ORG") or api.call("GET", "/v1/orgs")["data"][0]["id"]
    model = os.environ.get("EVERRUNS_MODEL", DEFAULT_MODEL)
    model_id = next(
        (m["id"] for m in api.call("GET", "/v1/models")["data"] if m.get("model_id") == model),
        None,
    )
    if not model_id:
        raise SystemExit(f"Model {model} is not configured on this server.")

    connection_id = provider_connection(api, org, args.provider)
    fields, system_prompt = read_agent_file()
    definition = {
        "name": fields["name"],
        "description": fields.get("description"),
        "system_prompt": system_prompt,
        "default_model_id": model_id,
        "sandbox_policy": sandbox_policy(args.provider, connection_id),
    }
    # Rerunning the example updates the agent it created the first time.
    existing = next(
        (a for a in api.call("GET", "/v1/agents?limit=200")["data"] if a["name"] == fields["name"]),
        None,
    )
    if existing:
        agent = api.call("PUT", f"/v1/agents/{existing['id']}", definition)
    else:
        agent = api.call("POST", "/v1/agents", definition)
    print(f"Agent: {agent['id']} ({agent['name']}), model {model}")

    session = api.call(
        "POST",
        "/v1/sessions",
        {"agent_id": agent["id"], "title": f"Review @{args.handle} on X"},
    )
    print(f"Session: {session['id']}")

    prompt = (
        f"Review the recent posts of the X account @{args.handle} "
        f"(https://x.com/{args.handle}). Write the review to {REVIEW_PATH} "
        "in your workspace, then reply with the same review."
    )
    api.call(
        "POST",
        f"/v1/sessions/{session['id']}/messages",
        {"message": {"role": "user", "content": [{"type": "text", "text": prompt}]}},
    )
    print(f"\nUser: {prompt}\n")
    wait_for_turn(api, session["id"])
    print("\nAgent:\n" + last_assistant_text(api, session["id"]))


if __name__ == "__main__":
    sys.exit(main())
