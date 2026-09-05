#!/usr/bin/env python3
"""Opt-in, single-host Responses steering owner. See README.md for the boundary.

The socket never leaves its owner. Producers only append durable commands. There
is deliberately no automatic replay across the send/ack crash window: OpenAI
provides neither a client steering idempotency key nor successor discovery.
"""

import argparse
import asyncio
import fcntl
import json
import os
import sqlite3
import sys
import uuid
from contextlib import contextmanager
from pathlib import Path
from urllib.parse import quote
from urllib.request import Request, urlopen

TERMINAL = {"completed", "incomplete", "failed", "cancelled"}
UNCOMMITTED = {"sending", "accepted", "pending", "uncertain"}
OUTPUT_TYPES = {
    "function_call": "function_call_output",
    "custom_tool_call": "custom_tool_call_output",
    "mcp_approval_request": "mcp_approval_response",
}


def result_key(item):
    return item["type"] + ":" + item.get("call_id", item.get("approval_request_id", ""))


def required_from(response):
    required = []
    for item in response.get("output", []):
        if item["type"] in OUTPUT_TYPES:
            stub = {"type": OUTPUT_TYPES[item["type"]]}
            if item["type"] == "mcp_approval_request":
                stub["approval_request_id"] = item["id"]
            else:
                stub["call_id"] = item["call_id"]
            required.append(stub)
    return required


def validate_result(item, required):
    if result_key(item) not in {result_key(r) for r in required}:
        raise ValueError("Result does not match an outstanding tool or approval")
    if item["type"] == "mcp_approval_response":
        if type(item.get("approve")) is not bool:
            raise ValueError("Approval requires an explicit boolean approve")
        allowed = {"type", "approval_request_id", "approve", "reason"}
    elif item["type"] in {"function_call_output", "custom_tool_call_output"}:
        if not isinstance(item.get("output"), str):
            raise ValueError("This prototype accepts string tool outputs")
        allowed = {"type", "call_id", "output"}
    else:
        raise ValueError("Unsupported required input; preserved for operator handling")
    if set(item) - allowed:
        raise ValueError("Unexpected result fields")


def initial_state(prompt, settings):
    # Keep the experiment on the documented compatible surface. Normal HTTP
    # drivers and globally configured models never consult this example.
    allowed = {"instructions", "tools", "reasoning", "max_output_tokens"}
    if set(settings) - allowed:
        raise ValueError(
            "Settings must be instructions, tools, reasoning, max_output_tokens"
        )
    if settings.get("reasoning", {}).get("effort", "medium") not in {
        "low",
        "medium",
        "high",
        "xhigh",
    }:
        raise ValueError("Unsupported reasoning effort for this prototype")
    for tool in settings.get("tools", []):
        if tool.get("type") not in {"function", "custom", "mcp"} or tool.get("async"):
            raise ValueError(
                "Only synchronous function/custom tools and MCP are supported"
            )
    return {
        "session": str(uuid.uuid4()),
        "prompt": prompt,
        "settings": {
            "model": "gpt-6-astra",
            "store": True,
            "reasoning": {"effort": "medium"},
            **settings,
        },
        "phase": "new",
        "current": None,
        "responses": {},
        "intents": [],
        "required": [],
        "create": None,
        "error": None,
        "owner": None,
    }


class Journal:
    def __init__(self, path):
        self.path = Path(path).resolve()
        self.db = sqlite3.connect(self.path, timeout=10)
        self.db.execute("PRAGMA synchronous=FULL")

    @classmethod
    def create(cls, path, prompt, settings):
        state = initial_state(prompt, settings)
        fd = os.open(path, os.O_CREAT | os.O_EXCL | os.O_WRONLY, 0o600)
        os.close(fd)
        journal = cls(path)
        with journal.db:
            journal.db.executescript("""
                CREATE TABLE state (id INTEGER PRIMARY KEY CHECK(id=1), value TEXT NOT NULL);
                CREATE TABLE inbox (id TEXT PRIMARY KEY, text TEXT NOT NULL, consumed INTEGER DEFAULT 0);
                CREATE TABLE results (id TEXT PRIMARY KEY, value TEXT NOT NULL);
                CREATE TABLE events (id INTEGER PRIMARY KEY, value TEXT NOT NULL);
            """)
            journal.db.execute("INSERT INTO state VALUES (1, ?)", (json.dumps(state),))
        return journal

    def load(self):
        return json.loads(
            self.db.execute("SELECT value FROM state WHERE id=1").fetchone()[0]
        )

    def save(self, state, event=None):
        with self.db:
            self.db.execute("UPDATE state SET value=? WHERE id=1", (json.dumps(state),))
            if event is not None:
                self.db.execute(
                    "INSERT INTO events(value) VALUES (?)", (json.dumps(event),)
                )

    def enqueue(self, local_id, text):
        if not text.strip():
            raise ValueError("Update must not be empty")
        with self.db:
            self.db.execute(
                "INSERT OR IGNORE INTO inbox(id,text) VALUES (?,?)", (local_id, text)
            )
            existing = self.db.execute(
                "SELECT text FROM inbox WHERE id=?", (local_id,)
            ).fetchone()[0]
            if existing != text:
                raise ValueError("Input id already belongs to a different update")

    def drain(self, state):
        with self.db:
            for local_id, text in self.db.execute(
                "SELECT id,text FROM inbox WHERE consumed=0 ORDER BY rowid"
            ).fetchall():
                state["intents"].append(
                    {"id": local_id, "text": text, "status": "queued"}
                )
                self.db.execute("UPDATE inbox SET consumed=1 WHERE id=?", (local_id,))
            self.db.execute("UPDATE state SET value=? WHERE id=1", (json.dumps(state),))

    def submit_result(self, item):
        key = result_key(item)
        existing = self.db.execute(
            "SELECT value FROM results WHERE id=?", (key,)
        ).fetchone()
        if existing:
            if json.loads(existing[0]) != item:
                raise ValueError("A different result was already saved for this call")
            return
        validate_result(item, self.load()["required"])
        with self.db:
            self.db.execute(
                "INSERT OR IGNORE INTO results VALUES (?,?)", (key, json.dumps(item))
            )
            saved = json.loads(
                self.db.execute(
                    "SELECT value FROM results WHERE id=?", (key,)
                ).fetchone()[0]
            )
            if saved != item:
                raise ValueError("A different result was already saved for this call")

    def results(self, required):
        items = []
        for stub in required:
            row = self.db.execute(
                "SELECT value FROM results WHERE id=?", (result_key(stub),)
            ).fetchone()
            if row is None:
                return None
            items.append(json.loads(row[0]))
        return items

    @contextmanager
    def ownership(self):
        # An OS lock does not expire under a paused owner. Same-host workers can
        # submit via SQLite, but a second owner cannot open another live socket.
        fd = os.open(str(self.path) + ".owner", os.O_CREAT | os.O_RDWR, 0o600)
        try:
            fcntl.flock(fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
            yield
        finally:
            os.close(fd)


class Owner:
    def __init__(self, journal):
        self.journal = journal
        self.state = journal.load()

    def save(self, event=None):
        self.journal.save(self.state, event)

    def uncommitted(self):
        return [i for i in self.state["intents"] if i["status"] in UNCOMMITTED]

    def create(self, inputs, parent=None, intent=None):
        request = {
            **self.state["settings"],
            "type": "response.create",
            "input": inputs,
            "metadata": {"everruns_steering_session": self.state["session"]},
        }
        if parent:
            request["previous_response_id"] = parent
        self.state["create"] = {"parent": parent, "intent": intent}
        self.state["phase"] = "creating"
        # Persist before send. A crash here is ambiguous, never a reason to retry.
        self.save({"outbound": request})
        return request

    def next_request(self):
        s = self.state
        self.journal.drain(s)
        if s["phase"] == "new":
            return self.create(s["prompt"])
        if s["phase"] in {"reconcile", "failed", "creating"}:
            return None
        current = s["responses"].get(s["current"], {})
        if current.get("status") in TERMINAL and s["required"]:
            # Wait for the pending event if a steer is awaiting acknowledgement.
            if any(i["status"] in {"sending", "accepted"} for i in self.uncommitted()):
                return None
            results = self.journal.results(s["required"])
            if results is not None:
                return self.create(results, s["current"])
            return None
        if self.uncommitted():
            return None
        queued = next((i for i in s["intents"] if i["status"] == "queued"), None)
        if queued is None:
            return None
        if s["phase"] == "active":
            queued.update(status="sending", parent=s["current"])
            request = {
                "type": "response.steer",
                "previous_response_id": s["current"],
                "input": queued["text"],
            }
            self.save({"outbound": request, "local_id": queued["id"]})
            return request
        if s["phase"] == "complete":
            queued.update(status="sending", parent=s["current"])
            return self.create(
                [{"role": "user", "content": queued["text"]}],
                s["current"],
                queued["id"],
            )
        return None

    def event(self, event):
        s = self.state
        kind = event["type"]
        if event.get("stream_id"):
            raise ValueError(
                "Unexpected named lane on dedicated default-lane connection"
            )
        if kind == "response.created":
            response = event["response"]
            rid = response["id"]
            if rid in s["responses"]:
                return  # A repeated event must not commit a newly queued update.
            parent = response.get("previous_response_id")
            expected = s["create"]
            if parent != s["current"] or (expected and parent != expected["parent"]):
                raise ValueError("Successor does not belong to this response chain")
            if parent and s["responses"][parent].get("status") not in TERMINAL:
                raise ValueError("Overlapping responses on the owned lane")
            if not expected and not self.uncommitted():
                raise ValueError("Unexpected automatic successor")
            for intent in self.uncommitted():
                if intent.get("parent") == parent:
                    if intent["status"] == "sending" and not (
                        expected and expected["intent"] == intent["id"]
                    ):
                        raise ValueError(
                            "Successor arrived before steering acknowledgement"
                        )
                    intent.update(status="applied", successor=rid)
            s["responses"][rid] = response
            s.update(current=rid, phase="active", required=[], create=None)
        elif kind.startswith("response.steer."):
            steer = event["steer"]
            candidates = [
                i
                for i in s["intents"]
                if steer.get("id") and i.get("steer_id") == steer["id"]
            ]
            if not candidates:
                candidates = [
                    i
                    for i in s["intents"]
                    if i["status"] == "sending"
                    and i.get("parent") == steer["previous_response_id"]
                ]
            if len(candidates) != 1:
                raise ValueError("Cannot correlate steering acknowledgement")
            intent = candidates[0]
            if kind == "response.steer.accepted":
                if intent["status"] == "sending":
                    intent.update(status="accepted", steer_id=steer["id"])
            elif kind == "response.steer.pending":
                if intent["status"] not in {"accepted", "pending"}:
                    raise ValueError("Pending event without accepted input")
                intent["status"] = "pending"
                s["required"] = list(
                    {result_key(r): r for r in event["required_input"]}.values()
                )
                s["phase"] = "waiting"
            elif kind == "response.steer.failed":
                if steer["input"] != intent["text"] or intent["status"] == "applied":
                    raise ValueError(
                        "Failed input differs from the uncommitted submission"
                    )
                intent.update(status="failed", error=event["error"])
                self.finish_if_ready()
        elif kind in {"response.completed", "response.incomplete", "response.failed"}:
            response = event["response"]
            if response["id"] not in s["responses"]:
                raise ValueError("Terminal event for an unknown response")
            # Replace by response id, never add terminal usage on event receipt.
            s["responses"][response["id"]] = response
            if response["id"] == s["current"]:
                s["required"] = required_from(response)
                self.finish_if_ready()
        elif kind == "error":
            s.update(phase="reconcile", error=event["error"])
        self.save(event)

    def finish_if_ready(self):
        s = self.state
        response = s["responses"].get(s["current"], {})
        status = response.get("status")
        if status not in TERMINAL:
            return
        steered = (
            response.get("incomplete_details", {}).get("reason") == "steered"
            if response.get("incomplete_details")
            else False
        )
        if status in {"failed", "cancelled"} or (
            status == "incomplete" and not steered
        ):
            s.update(
                phase="waiting" if self.uncommitted() else "failed",
                error=response.get("error") or response.get("incomplete_details"),
            )
        elif self.uncommitted() or s["required"]:
            s["phase"] = "waiting"
        elif steered:
            s.update(
                phase="reconcile", error="Steered response without a known successor"
            )
        else:
            s["phase"] = "complete"

    def disconnected(self, reason):
        s = self.state
        s["owner"] = None
        if s["phase"] not in {"new", "complete", "failed"} or self.uncommitted():
            for intent in self.uncommitted():
                intent["status"] = "uncertain"
            s.update(phase="reconcile", error=s["error"] or str(reason))
        self.save({"disconnected": str(reason)})


def usage_totals(state):
    def add(target, source):
        for key, value in source.items():
            if isinstance(value, dict):
                add(target.setdefault(key, {}), value)
            elif isinstance(value, (int, float)):
                target[key] = target.get(key, 0) + value

    total = {}
    for response in state["responses"].values():
        if response.get("status") in TERMINAL:
            add(total, response.get("usage") or {})
    return total


def get_json(path, key, base="https://api.openai.com/v1"):
    request = Request(base + path, headers={"Authorization": "Bearer " + key})
    with urlopen(request, timeout=30) as response:
        return json.load(response)


def input_items(rid, key, base):
    items, after = [], ""
    while True:
        page = get_json(
            "/responses/"
            + quote(rid, safe="")
            + "/input_items?order=asc&limit=100"
            + after,
            key,
            base,
        )
        items.extend(page["data"])
        if not page.get("has_more"):
            return items
        after = "&after=" + quote(page["last_id"], safe="")


def user_texts(items):
    values = []
    for item in items:
        if item.get("role") == "user":
            content = item["content"]
            values.append(
                content
                if isinstance(content, str)
                else "".join(p["text"] for p in content if p["type"] == "input_text")
            )
    return values


def reconcile(owner, key, successor_id=None, base="https://api.openai.com/v1"):
    """Recover only provider-proven history. Absence of a successor is not proof."""
    s = owner.state
    if successor_id:
        child = get_json("/responses/" + quote(successor_id, safe=""), key, base)
        parent = s["current"]
        if (
            child.get("id") != successor_id
            or child.get("previous_response_id") != parent
            or child.get("metadata", {}).get("everruns_steering_session")
            != s["session"]
        ):
            raise ValueError("Recovery response has wrong parent or session")
        pending = owner.uncommitted()
        expected_create = s["create"]
        old = user_texts(input_items(parent, key, base)) if parent else []
        history = input_items(successor_id, key, base)
        new = user_texts(history)
        expected = [i["text"] for i in pending]
        if expected_create and parent is None:
            expected = [s["prompt"]]
        if (not pending and not expected_create) or new != old + expected:
            raise ValueError(
                "History does not prove exactly one copy of each unresolved input"
            )
        if expected_create and s["required"]:
            saved = owner.journal.results(s["required"])
            if saved is None:
                raise ValueError(
                    "Missing saved results for the interrupted continuation"
                )
            for result in saved:
                matches = [
                    item
                    for item in history
                    if item.get("type") == result["type"]
                    and result_key(item) == result_key(result)
                ]
                if len(matches) != 1 or any(
                    matches[0].get(k) != v for k, v in result.items()
                ):
                    raise ValueError(
                        "History does not prove commitment of saved tool results"
                    )
        for intent in pending:
            intent.update(status="applied", successor=successor_id)
        s["responses"][successor_id] = child
        s.update(current=successor_id, create=None)
        owner.save({"reconciled_successor": successor_id})
    if owner.uncommitted() or s["create"]:
        s.update(
            phase="reconcile",
            error="Unknown send/commit outcome; supply a verified successor id. No input replayed.",
        )
        owner.save()
        return False
    for rid, response in list(s["responses"].items()):
        if response.get("status") not in TERMINAL:
            fresh = get_json("/responses/" + quote(rid, safe=""), key, base)
            if fresh["id"] != rid:
                raise ValueError("Recovery response id mismatch")
            s["responses"][rid] = fresh
    response = s["responses"].get(s["current"], {})
    if response.get("status") not in TERMINAL:
        s.update(
            phase="reconcile",
            error="Known response still running; poll recovery. Cannot steer it from a new socket.",
        )
        owner.save()
        return False
    s["required"] = required_from(response)
    s.update(phase="waiting", error=None)
    owner.finish_if_ready()
    owner.save({"recovered": s["current"]})
    return s["phase"] != "reconcile"


async def run_owner(
    journal,
    key,
    endpoint="wss://api.openai.com/v1/responses",
    stop_when_idle=False,
    timeout=3500,
):
    with journal.ownership():
        owner = Owner(journal)
        if owner.state["phase"] != "new":
            raise ValueError("Use recover before resume, or init a fresh journal")
        await connected_owner(owner, key, endpoint, stop_when_idle, timeout)


async def connected_owner(owner, key, endpoint, stop_when_idle, timeout):
    from websockets.asyncio.client import connect

    owner.state["owner"] = {"pid": os.getpid(), "connection": str(uuid.uuid4())}
    owner.save()
    try:
        async with (
            asyncio.timeout(timeout),
            connect(
                endpoint,
                additional_headers={"Authorization": "Bearer " + key},
                open_timeout=10,
                max_size=16 * 1024 * 1024,
            ) as socket,
        ):
            while True:
                request = owner.next_request()
                if request:
                    await socket.send(json.dumps(request))
                if owner.state["phase"] in {"failed", "reconcile"}:
                    return
                if (
                    stop_when_idle
                    and owner.state["phase"] == "complete"
                    and not any(i["status"] == "queued" for i in owner.state["intents"])
                ):
                    return
                try:
                    raw = await asyncio.wait_for(socket.recv(), 0.1)
                except TimeoutError:
                    continue
                event = json.loads(raw)
                owner.event(event)
                print(
                    json.dumps(
                        {
                            "event": event["type"],
                            "phase": owner.state["phase"],
                            "response": owner.state["current"],
                            "intents": [
                                {"id": i["id"], "status": i["status"]}
                                for i in owner.state["intents"]
                            ],
                        }
                    ),
                    flush=True,
                )
    finally:
        owner.disconnected("Owner stopped; inspect journal before recovery")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--db", required=True)
    commands = parser.add_subparsers(dest="command", required=True)
    init = commands.add_parser("init")
    init.add_argument("prompt")
    init.add_argument(
        "--settings",
        help="JSON file: synchronous tools, instructions, reasoning, limits",
    )
    update = commands.add_parser("update")
    update.add_argument("text")
    update.add_argument(
        "--id", required=True, help="Stable user-message id for producer retries"
    )
    result = commands.add_parser("result")
    result.add_argument(
        "json_file", help="Completed tool output or explicit approval JSON"
    )
    commands.add_parser("status")
    commands.add_parser("run")
    recover = commands.add_parser("recover")
    recover.add_argument("--successor-id")
    commands.add_parser("resume")
    args = parser.parse_args()
    if args.command == "init":
        settings = json.loads(Path(args.settings).read_text()) if args.settings else {}
        Journal.create(args.db, args.prompt, settings).db.close()
        return
    if not Path(args.db).is_file():
        raise ValueError("Journal does not exist; run init")
    journal = Journal(args.db)
    try:
        if args.command == "update":
            journal.enqueue(args.id, args.text)
        elif args.command == "result":
            journal.submit_result(json.loads(Path(args.json_file).read_text()))
        elif args.command == "status":
            state = journal.load()
            state["usage_totals"] = usage_totals(state)
            state["inbox"] = journal.db.execute(
                "SELECT id,text,consumed FROM inbox ORDER BY rowid"
            ).fetchall()
            print(json.dumps(state, indent=2))
        elif args.command == "run":
            asyncio.run(run_owner(journal, os.environ["OPENAI_API_KEY"]))
        else:
            with journal.ownership():
                owner = Owner(journal)
                recovered = reconcile(
                    owner,
                    os.environ["OPENAI_API_KEY"],
                    getattr(args, "successor_id", None),
                )
                if (
                    args.command == "resume"
                    and recovered
                    and owner.state["phase"] != "failed"
                ):
                    asyncio.run(
                        connected_owner(
                            owner,
                            os.environ["OPENAI_API_KEY"],
                            "wss://api.openai.com/v1/responses",
                            False,
                            3500,
                        )
                    )
                print(
                    json.dumps(
                        {"phase": owner.state["phase"], "error": owner.state["error"]}
                    )
                )
    finally:
        journal.db.close()


if __name__ == "__main__":
    try:
        main()
    except (ValueError, BlockingIOError, KeyError) as error:
        sys.exit(str(error))
