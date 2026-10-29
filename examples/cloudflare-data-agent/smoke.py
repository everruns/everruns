#!/usr/bin/env python3
"""Live CP -> deployed Worker -> real model/private D1 -> CP smoke."""
import argparse
import json
import ipaddress
import os
import subprocess
import threading
import time
import urllib.request
import urllib.error
import urllib.parse
import uuid


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cp", required=True)
    parser.add_argument("--endpoint", required=True)
    parser.add_argument("--org", help="Organization public ID when the account has multiple organizations")
    args = parser.parse_args()
    cp = urllib.parse.urlsplit(args.cp)
    try:
        loopback = ipaddress.ip_address(cp.hostname or "").is_loopback
    except ValueError:
        loopback = cp.hostname == "localhost"
    if cp.username or cp.password or cp.query or cp.fragment or not cp.hostname or (cp.scheme != "https" and not (cp.scheme == "http" and loopback)):
        parser.error("--cp requires HTTPS, or HTTP on loopback, without URL credentials/query/fragment")
    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, request, response, code, message, headers, target):
            return None
    opener = urllib.request.build_opener(NoRedirect())
    token = os.environ.get("EVERRUNS_TOKEN")
    secret = os.environ["EVERRUNS_ENDPOINT_SECRET"]

    def api(method, path, body=None):
        request = urllib.request.Request(args.cp.rstrip("/") + "/api/v1" + path,
            data=None if body is None else json.dumps(body).encode(), method=method,
            headers={"Content-Type": "application/json", **({"Authorization": "Bearer " + token} if token else {}), **({"X-Org-Id": args.org} if args.org else {})})
        try:
            with opener.open(request, timeout=150) as response:
                result = response.read()
                return json.loads(result) if result else None
        except urllib.error.HTTPError as error:
            try:
                problem = json.load(error)
                detail = str(problem.get("detail", problem.get("title", "API request failed")))[:512]
            except (ValueError, AttributeError):
                detail = "API request failed"
            raise RuntimeError(f"{method} {path}: HTTP {error.code}: {detail}") from None

    # Agent requires an installation-provided harness, just as in the UI.
    harnesses = api("GET", "/harnesses")
    harnesses = harnesses.get("data", harnesses) if isinstance(harnesses, dict) else harnesses
    active_harnesses = [h for h in harnesses if h.get("status", "active") == "active"]
    harness = next((h for h in active_harnesses if h["name"] == "conversation"), active_harnesses[0])
    agent = api("POST", "/agents", {"name": "cloudflare-smoke-" + uuid.uuid4().hex[:12], "display_name": "Cloudflare sales agent", "system_prompt": "External worker owns execution", "harness_id": harness["id"], "communication": "direct"})
    agent_id = agent["id"]
    path = "/agents/" + agent_id + "/external-worker"
    sessions = []
    sse = None
    live = []
    try:
        binding = api("POST", path, {"endpoint_url": args.endpoint, "endpoint_secret": secret})
        assert binding["enabled"] and "endpoint_secret" not in binding
        session = api("POST", "/sessions", {"agent_id": agent_id, "source": "playground"})
        session_id = session["id"]
        sessions.append(session_id)

        headers = {"Authorization": "Bearer " + token} if token else {}
        if args.org:
            headers["X-Org-Id"] = args.org
        config = "".join("header = " + json.dumps(k + ": " + v) + "\n" for k, v in headers.items())
        sse = subprocess.Popen(["curl", "--config", "-", "--silent", "--show-error", "--no-buffer", "--max-time", "300", args.cp.rstrip("/") + "/api/v1/sessions/" + session_id + "/sse?after_sequence=0"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        sse.stdin.write(config)
        sse.stdin.close()
        def collect():
            for line in sse.stdout:
                if line.startswith("data:"):
                    event = json.loads(line[5:].strip())
                    if event.get("type"): live.append(event)
        threading.Thread(target=collect, daemon=True).start()

        def invoke(text, expected_turns):
            api("POST", "/sessions/" + session_id + "/messages", {"message": {"role": "user", "content": [{"type": "text", "text": text}]}})
            deadline = time.monotonic() + 150
            while time.monotonic() < deadline:
                events = api("GET", "/sessions/" + session_id + "/events?limit=1000")
                events = events.get("data", events.get("events", [])) if isinstance(events, dict) else events
                assert not any(e["type"] == "turn.failed" for e in events), "Worker failed; inspect CP trace"
                if sum(e["type"] == "turn.completed" for e in events) >= expected_turns:
                    return events
                time.sleep(0.5)
            raise AssertionError("Timed out waiting for custom turn")

        events = invoke("What are north sales? Include exact total cents and order count.", 1)
        # Event API uses canonical type/data/context, including CP provenance.
        tools = [e for e in events if e["type"] == "tool.completed"]
        summaries = [json.loads(part["text"]) for event in tools if event["data"].get("tool_name") == "sales_summary" for part in event["data"].get("result", []) if part.get("type") == "text"]
        assert any(summary.get("total_cents") == 27278 and summary.get("order_count") == 2 for summary in summaries), "Missing actual D1 aggregate result"
        generations = [e for e in events if e["type"] == "llm.generation"]
        assert generations and any(e["data"]["metadata"].get("usage", {}).get("output_tokens", 0) > 0 for e in generations)
        traces = {e["context"]["trace_id"] for e in events if e["type"] in ("turn.completed", "tool.completed", "llm.generation")}
        assert len(traces) == 1, "Execution events did not share the canonical turn trace"
        outputs = [e for e in events if e["type"] == "output.message.completed"]
        assert "27278" in json.dumps(outputs).replace(",", "")
        for event in events:
            if event["type"] in ("turn.completed", "tool.completed", "llm.generation"):
                assert event["context"].get("trace_id")
                context = event["context"]
                assert context.get("turn_id") != context["input_message_id"].replace("message_", "turn_"), "Missing CP turn namespacing"
                if event["type"] == "turn.completed":
                    assert event["data"]["turn_id"] == context["turn_id"]
        work = api("GET", "/sessions/" + session_id + "/work")
        assert "sales_summary" in json.dumps(work), "Tool missing from ordinary work projection"
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline and not any(e["type"] == "tool.completed" for e in live):
            time.sleep(0.1)
        assert any(e["type"] == "tool.completed" for e in live), "Missing live SSE tool event"
        assert any(e["type"] == "output.message.completed" for e in live), "Missing live SSE answer"
        events = invoke("Which region did I ask about in my previous message?", 2)
        assert "north" in json.dumps([e for e in events if e["type"] == "output.message.completed"][-1:]).lower()
        api("DELETE", path)
        assert not api("GET", path)["enabled"]
        api("POST", "/sessions/" + session_id + "/messages", {"message": {"role": "user", "content": [{"type": "text", "text": "This must remain queued."}]}})
        time.sleep(3)
        events = api("GET", "/sessions/" + session_id + "/events?limit=1000")
        events = events.get("data", events.get("events", [])) if isinstance(events, dict) else events
        assert sum(e["type"] == "turn.completed" for e in events) == 2, "Disabled external work fell back to another runtime"
        api("POST", path, {"endpoint_url": args.endpoint, "endpoint_secret": secret})
        deadline = time.monotonic() + 150
        while time.monotonic() < deadline:
            resumed = api("GET", "/sessions/" + session_id + "/events?limit=1000")["data"]
            if sum(e["type"] == "turn.completed" for e in resumed) == 3:
                break
            assert not any(e["type"] == "turn.failed" for e in resumed), "Queued input failed after re-registration"
            time.sleep(0.5)
        else:
            raise AssertionError("Re-registering did not resume the queued input")
        print(json.dumps({"result": "passed", "agent_id": agent_id, "sessions": sessions, "turns": 3, "checks": ["ownership", "private_d1", "real_model", "canonical_trace", "live_sse", "work_projection", "history", "revocation_no_fallback", "re_registration_resumes_queued_work"]}, indent=2))
    finally:
        if sse is not None:
            sse.terminate()
            sse.wait(timeout=5)
        api("DELETE", path)
        # Preserve traces for inspection; the disabled binding prevents execution.


if __name__ == "__main__":
    main()
