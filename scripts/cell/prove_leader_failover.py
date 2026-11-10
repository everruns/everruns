#!/usr/bin/env python3
"""Kill a cell's Raft leader while a model call is in flight on a follower.

Pass: the turn completes, its in-flight model call was not repeated (the
executor kept running across the leader change), and the restarted old leader
catches up with the cell.
"""
import json, os, subprocess, sys, time, urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "scripts", "cell"))
import fleet  # noqa: E402

DRIVE = os.path.join(ROOT, "target", "release", "cell-drive")

DELAY_MS = 3000


def call(method, url, body=None, timeout=60):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method, headers={"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.load(r)


def gw(cell, node):
    return f"http://{fleet.addr(cell, node)}"


def main():
    subprocess.run([sys.executable, os.path.join(ROOT, "scripts/cell/fleet.py"), "down"])
    time.sleep(0.5)
    subprocess.run(["rm", "-rf", fleet.STATE])
    os.environ["CELL_MODEL_DELAY_MS"] = str(DELAY_MS)
    fleet.up(1, 3, 7100)
    time.sleep(1)

    leader = fleet.leader(0)
    other = next(n for n in (1, 2, 3) if n != leader)
    base = gw(0, other)
    # The client owns the agent definition; the test driver prints it.
    slow = json.loads(subprocess.run([DRIVE, "agent", "slow"], capture_output=True,
                                     text=True, check=True).stdout)
    for attempt in range(5):
        session = call("POST", f"{base}/v1/sessions", {"agent": slow})["id"]
        call("POST", f"{base}/v1/sessions/{session}/messages", {"text": "go"})
        active = None
        for _ in range(100):
            view = call("GET", f"{base}/v1/sessions/{session}")
            if view["active"]:
                active = view["active"]
                break
            time.sleep(0.05)
        if active and active["executor"] != leader:
            break
        call("GET", f"{base}/v1/sessions/{session}/wait?turns=1")
    else:
        sys.exit("could not place a turn on a follower")

    executor = active["executor"]
    # Wait until the model call is in flight.
    while True:
        events = call("GET", f"{base}/v1/sessions/{session}/events")["data"]
        if any(e["type"] == "reason.started" for e in events):
            break
        time.sleep(0.05)
    seq_before = events[-1]["sequence"]
    time.sleep(0.8)
    killed_at = time.time()
    fleet.kill(0, leader)
    print(f"killed leader {leader} during a model call executing on node {executor}")

    survivor = gw(0, executor)
    new_leader = None
    while new_leader is None or new_leader == leader:
        try:
            new_leader = fleet.stats(0, executor)["leader"]
        except Exception:
            pass
        time.sleep(0.05)
    elected = time.time() - killed_at
    view = call("GET", f"{survivor}/v1/sessions/{session}/wait?turns=1&timeout_ms=60000", timeout=90)
    events = call("GET", f"{survivor}/v1/sessions/{session}/events")["data"]
    first_after = next((e for e in events if e["sequence"] > seq_before), None)

    reasons = sum(1 for e in events if e["type"] == "reason.started")
    completed = sum(1 for e in events if e["type"] == "turn.completed")
    turn_started = sum(1 for e in events if e["type"] == "turn.started")

    fleet.start(0, leader)
    caught_up = False
    for _ in range(200):
        try:
            applied = {n: fleet.stats(0, n)["applied"] for n in (1, 2, 3)}
            if len(set(applied.values())) == 1:
                caught_up = True
                break
        except Exception:
            pass
        time.sleep(0.1)

    result = {
        "killed_leader": leader,
        "executor": executor,
        "new_leader": new_leader,
        "election_secs": round(elected, 3),
        "turns_completed": view["turns_completed"],
        "turn_started_events": turn_started,
        "turn_completed_events": completed,
        "model_calls_started": reasons,
        "expected_model_calls": 2,
        "events": len(events),
        "restarted_leader_caught_up": caught_up,
    }
    result["ok"] = (view["turns_completed"] == 1 and completed == 1 and turn_started == 1
                    and reasons == 2 and caught_up)
    print(json.dumps(result, indent=2))
    sys.exit(0 if result["ok"] else 1)


if __name__ == "__main__":
    main()
