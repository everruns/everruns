#!/usr/bin/env python3
"""Large inputs: user messages and tool results of growing size.

Sends one message of each size to a fresh session (agent `chat`), or runs
one turn per session whose tool returns `CELL_BLOB_KB` (agent `blob`),
then a small follow-up turn, and reports what happened. Needs a running
fleet (`scripts/cell/fleet.py up`).
"""
import argparse, json, subprocess, time, urllib.request, urllib.error

GW = "http://127.0.0.1:7101"

def call(method, path, body=None, timeout=120):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(GW + path, data=data, method=method,
                                 headers={"content-type": "application/json"})
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()[:200]
    except (urllib.error.URLError, OSError) as e:
        return 599, str(e)[:200]

def agent(name):
    out = subprocess.check_output(["target/release/cell-drive", "agent", name])
    return json.loads(out)

def turn(sid, text, n, wait_s):
    t = time.time()
    status, body = call("POST", f"/v1/sessions/{sid}/messages", {"text": text})
    if status >= 300:
        return {"admit": status, "error": body}
    deadline = time.time() + wait_s
    while time.time() < deadline:
        _, view = call("GET", f"/v1/sessions/{sid}/wait?turns={n}&timeout_ms=10000")
        if isinstance(view, dict) and view.get("turns_completed", 0) >= n:
            return {"admit": status, "secs": round(time.time() - t, 2)}
    return {"admit": status, "timeout": wait_s}

def main():
    p = argparse.ArgumentParser()
    p.add_argument("--agent", default="chat")
    p.add_argument("--mb", default="0.5,1,1.9,2.5,8,32")
    p.add_argument("--turns", type=int, default=1, help="large turns per session")
    p.add_argument("--wait", type=int, default=120)
    a = p.parse_args()
    definition = agent(a.agent)
    for mb in [float(x) for x in a.mb.split(",")]:
        _, s = call("POST", "/v1/sessions", {"agent": definition})
        sid = s["id"]
        text = ("lorem ipsum dolor sit amet " * int(mb * 1024 * 1024 / 27 + 1))[: int(mb * 1024 * 1024)]
        if a.agent != "chat":
            text = "go"
        results = [turn(sid, text, n, a.wait) for n in range(1, a.turns + 1)]
        done = sum(1 for r in results if r.get("admit", 500) < 300)
        follow = turn(sid, "and now?", done + 1, a.wait)
        _, events = call("GET", f"/v1/sessions/{sid}/events")
        n = len(events) if isinstance(events, list) else (events.get("events") and len(events["events"]))
        failed = [e.get("type") or e.get("event_type") for e in (events if isinstance(events, list) else events.get("events", []))
                  if "failed" in str(e.get("type") or e.get("event_type"))]
        print(json.dumps({"mb": mb, "turns": results, "follow_up": follow, "events": n, "failed": failed[:3]}), flush=True)

if __name__ == "__main__":
    main()
