#!/usr/bin/env python3
"""A controller for agentd on its own: the app that owns customers, keys and
limits, using agentd as the backend that keeps sessions and runs turns.

It pushes the model key and the agents in ./agents, gives each customer a
spend limit, runs a conversation per customer, then reads the stream to
report what each customer's turns cost. Standard library only.

    agentd serve --dev --group-label customer      # or the Docker setup in README.md
    OPENAI_API_KEY=... python3 controller.py
"""
import json
import os
import pathlib
import sys
import urllib.error
import urllib.request

URL = os.environ.get("AGENTD_URL", "http://127.0.0.1:7700").rstrip("/")
KEY = os.environ.get("AGENTD_API_KEY")  # unset against `serve --dev`
HERE = pathlib.Path(__file__).parent


def call(method, path, body=None):
    request = urllib.request.Request(
        URL + path,
        method=method,
        data=None if body is None else json.dumps(body).encode(),
        headers={"content-type": "application/json"},
    )
    if KEY:
        request.add_header("authorization", f"Bearer {KEY}")
    try:
        with urllib.request.urlopen(request, timeout=120) as response:
            text = response.read().decode()
    except urllib.error.HTTPError as error:
        sys.exit(f"{method} {path}: {error.code} {error.read().decode()}")
    return json.loads(text) if text else None


def agents():
    """The agent files: one JSON definition per agent, its id the file name
    (the layout `agentd push ./agents` reads)."""
    for path in sorted((HERE / "agents").glob("*.json")):
        yield path.stem, json.loads(path.read_text())


def main():
    openai_key = os.environ.get("OPENAI_API_KEY") or sys.exit("set OPENAI_API_KEY")

    # The model key, bound to the provider the agents name. Values go in and
    # never come back out: GET /v1/secrets lists names only.
    call("PUT", "/v1/secrets/openai", {"value": openai_key, "bind": {"model": "openai"}})
    for agent_id, definition in agents():
        pushed = call("PUT", f"/v1/agents/{agent_id}", definition)
        print(f"agent {agent_id}: revision {pushed['revision']}")

    # Each customer is a group: its own spend limit, its own sessions.
    customers = {
        "acme": "My rear brake squeaks. How long would a repair take?",
        "globex": "Are you open on Mondays?",
    }
    for customer in customers:
        call("PUT", f"/v1/groups/{customer}", {"spend_usd": 0.50})

    for customer, question in customers.items():
        triage = call("POST", "/v1/sessions", {"agent_id": "triage", "labels": {"customer": customer}})
        kind = call("POST", f"/v1/sessions/{triage['id']}/messages?wait=true", {"text": question})
        session = call(
            "POST",
            "/v1/sessions",
            {"agent_id": "support", "labels": {"customer": customer, "topic": reply(kind).strip()}},
        )
        answer = call("POST", f"/v1/sessions/{session['id']}/messages?wait=true", {"text": question})
        print(f"\n[{customer}] ({reply(kind).strip()}) {question}\n  -> {reply(answer)}")

    # One stream for everything: here, what each customer's turns cost.
    cost, cursor = {}, ""
    while True:
        page = call("GET", f"/v1/stream?type=Usage&after={cursor}")
        for record in page["data"]:
            customer = record["labels"].get("customer", "?")
            cost[customer] = cost.get(customer, 0) + record["cost_micros"]
        cursor = page["cursor"]
        if not page["data"]:
            break
    print()
    for customer, micros in sorted(cost.items()):
        group = call("GET", f"/v1/groups/{customer}")
        print(f"{customer}: ${micros / 1e6:.4f} this run, ${group['spent_usd']:.4f} of ${group['spend_usd']:.2f} spent")


def reply(waited):
    """The assistant's text from a `wait=true` answer."""
    texts = []

    def walk(value):
        if isinstance(value, dict):
            if value.get("type") == "text" and isinstance(value.get("text"), str):
                texts.append(value["text"])
            for item in value.values():
                walk(item)
        elif isinstance(value, list):
            for item in value:
                walk(item)

    walk(waited.get("messages", waited))
    return "\n".join(texts) or json.dumps(waited)[:400]


if __name__ == "__main__":
    main()
