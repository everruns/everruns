#!/usr/bin/env python3
"""Exercise a real Serve + Bashkit app. Standard library only; model calls are billed."""

import argparse
import json
import os
from pathlib import Path
import secrets
import signal
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request


class Demo:
    def __init__(self, binary):
        self.binary = str(Path(binary).resolve(strict=True))
        self.root = Path(tempfile.mkdtemp(prefix="serve-bashkit-"))
        self.token = secrets.token_urlsafe(32)
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            self.port = listener.getsockname()[1]
        self.base = f"http://127.0.0.1:{self.port}"
        # Local HTTP must not go through a configured outbound proxy.
        self.http = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        self.process = None
        self.log = None
        self.cursor = {}

    def request(self, path, body=None):
        request = urllib.request.Request(
            self.base + path,
            data=None if body is None else json.dumps(body).encode(),
            headers={
                "Content-Type": "application/json",
                "Authorization": f"Bearer {self.token}",
            },
        )
        with self.http.open(request, timeout=10) as response:
            return json.load(response)

    def start(self):
        env = dict(os.environ, SERVE_DATA_DIR=str(self.root / "data"),
                   SERVE_API_KEYS=self.token)
        # This demo must never acquire a lease on an ambient remote store.
        env.pop("SERVE_STORE", None)
        self.log = (self.root / "server.log").open("a")
        self.process = subprocess.Popen(
            [self.binary, "start", "--port", str(self.port)],
            env=env, stdout=self.log, stderr=subprocess.STDOUT,
        )
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if self.process.poll() is not None:
                raise RuntimeError(f"Serve exited; see {self.root / 'server.log'}")
            try:
                self.request("/health")
                print(f"Serve ready at {self.base}", flush=True)
                return
            except (urllib.error.URLError, TimeoutError):
                time.sleep(0.1)
        raise TimeoutError("Serve did not become ready in 30 seconds")

    def stop(self):
        if self.process is not None and self.process.poll() is None:
            self.process.send_signal(signal.SIGINT)
            try:
                self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
                raise RuntimeError("Serve did not stop gracefully")
        if self.log is not None:
            self.log.close()

    def session(self):
        session = self.request("/v1/sessions", {})["id"]
        self.cursor[session] = 0
        return session

    def turn(self, session, label, command, expected):
        print(f"\n{label}\n$ {command}", flush=True)
        self.request(f"/v1/sessions/{session}/messages", {
            "message": {"content": [{"type": "text", "text":
                "Run exactly this command using bash with output full, then stop:\n" + command}]}
        })
        events = []
        deadline = time.monotonic() + 180
        while time.monotonic() < deadline:
            batch = self.request(
                f"/v1/sessions/{session}/events?after_sequence={self.cursor[session]}"
            )["data"]
            events.extend(batch)
            if batch:
                self.cursor[session] = max(event["sequence"] for event in batch)
            terminal = next((event for event in batch if event["type"] in
                             ("turn.completed", "turn.failed", "turn.cancelled")), None)
            if terminal:
                break
            time.sleep(0.25)
        else:
            raise TimeoutError(f"{label}: turn exceeded 180 seconds")
        (self.root / f"{label}.json").write_text(json.dumps(events, indent=2) + "\n")
        if terminal["type"] != "turn.completed":
            raise RuntimeError(f"{label}: {terminal['type']}; inspect saved events")
        commands = [event["data"]["tool_call"]["arguments"].get("commands")
                    for event in events if event["type"] == "tool.started"
                    and event["data"]["tool_call"]["name"] == "bash"]
        if commands != [command]:
            raise RuntimeError(f"{label}: expected exactly the requested command, got {commands}")
        calls = [event["data"] for event in events
                 if event["type"] == "tool.completed" and event["data"]["tool_name"] == "bash"]
        if not calls:
            raise RuntimeError(f"{label}: no real bash tool result")
        stdout = ""
        for call in calls:
            if not call["success"]:
                raise RuntimeError(f"{label}: bash failed: {call.get('error')}")
            for part in call.get("result", []):
                if part["type"] == "text":
                    result = json.loads(part["text"])
                    if result["exit_code"] != 0:
                        raise RuntimeError(f"{label}: shell failed: {result}")
                    stdout += result["stdout"]
        if stdout != expected:
            raise RuntimeError(f"{label}: expected {expected!r}, got {stdout!r}")
        print(stdout, end="", flush=True)

    def run(self):
        print(f"Disposable data and evidence: {self.root}", flush=True)
        try:
            self.start()
            session = self.session()
            print(f"Session: {session}", flush=True)
            self.turn(session, "01-create",
                      "mkdir -p /workspace/demo && "
                      "printf 'apples\\npears\\napples\\n' > /workspace/demo/items.txt && "
                      "export DEMO_EPHEMERAL=only_this_call && "
                      "cat /workspace/demo/items.txt", "apples\npears\napples\n")
            report = "apples\npears\n"
            self.turn(session, "02-report",
                      "sort /workspace/demo/items.txt | uniq > /workspace/demo/report.txt && "
                      "cat /workspace/demo/report.txt && "
                      "printf 'variable=%s\\n' \"${DEMO_EPHEMERAL-unset}\"",
                      report + "variable=unset\n")
            self.stop()
            # Check disk, independently of the model's response and tool events.
            files = list((self.root / "data" / "workspaces").rglob("report.txt"))
            if len(files) != 1 or files[0].read_text() != report:
                raise RuntimeError(f"Unexpected persisted report files: {files}")
            print(f"Verified on disk: {files[0]}", flush=True)
            self.start()
            self.turn(session, "03-after-restart",
                      "cat /workspace/demo/report.txt", report)
            self.turn(self.session(), "04-isolation",
                      "if test -e /workspace/demo/items.txt || test -e /workspace/demo/report.txt; "
                      "then echo unexpected-files; exit 1; else echo isolated; fi", "isolated\n")
            print("\nPASS: files survive turns and restart; sessions are isolated; "
                  "shell variables reset.", flush=True)
        finally:
            self.stop()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", help="path to the built bashkit-workspace binary")
    args = parser.parse_args()
    Demo(args.binary).run()
