#!/usr/bin/env python3
"""A follower that fell behind the leader's log heals the cell when it must.

  prove_follower_catchup.py [--sessions 42000]

Starts one cell (a snapshot every 500 entries keeping 200 behind it, so
the leader purges its log soon), fills it with `--sessions` sessions so the snapshot is large, then
pauses one follower (SIGSTOP) while more load runs until the leader has
moved well past the follower's last entry (so it can only catch up by
snapshot). It then resumes that follower and kills the other one: the
cell has a quorum only once the lagging follower has the snapshot.

Pass: the lagging follower catches up, and a new turn completes, within
the deadline. Found by the 75k-session run: a follower catching up by a
150 MB snapshot timed out between chunks, called an election, cancelled
its own catch-up, and the cell never committed again.
"""
import argparse, json, os, signal, subprocess, sys, time, urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "scripts", "cell"))
import fleet  # noqa: E402

DRIVE = os.path.join(ROOT, "target", "release", "cell-drive")
DEADLINE_SECS = 180


def call(method, url, body=None, timeout=60):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method,
                                 headers={"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.load(r)


def pid_of(node):
    with open(fleet.pidfile(0, node)) as f:
        return int(f.read())


def load(gateway, sessions, turns):
    out = subprocess.run([DRIVE, "load", "--gateways", gateway, "--agent", "worker",
                          "--sessions", str(sessions), "--turns", str(turns),
                          "--concurrency", "1000"], capture_output=True, text=True)
    return json.loads(out.stdout) if out.stdout.strip() else {"error": out.stderr[-500:]}


def applied(node):
    try:
        return fleet.stats(0, node)["applied"]
    except Exception:
        return None


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--sessions", type=int, default=42000)
    a = p.parse_args()

    subprocess.run([sys.executable, os.path.join(ROOT, "scripts/cell/fleet.py"), "down"])
    time.sleep(0.5)
    subprocess.run(["rm", "-rf", fleet.STATE])
    os.environ["CELL_NODE_ARGS"] = "--snapshot-every 500 --keep-log 200"
    os.environ.setdefault("CELL_WORK_MS", "20")
    fleet.up(1, 3, 7100)
    time.sleep(1)

    leader = fleet.leader(0)
    lagging, other = [n for n in (1, 2, 3) if n != leader]
    gateway = f"http://{fleet.addr(0, leader)}"
    filled = load(gateway, a.sessions, 1)
    print(f"filled: {filled.get('turns_completed')} turns, leader {leader}", flush=True)

    os.kill(pid_of(lagging), signal.SIGSTOP)
    paused_at = applied(leader)
    print(f"paused node {lagging} at leader index {paused_at}", flush=True)
    # Enough new entries that the leader purges past the paused follower
    # (it keeps 200 behind each snapshot, taken every 500).
    while applied(leader) - paused_at < 1500:
        load(gateway, 500, 1)
    behind = applied(leader) - paused_at
    print(f"leader moved {behind} entries past the paused follower", flush=True)

    os.kill(pid_of(lagging), signal.SIGCONT)
    fleet.kill(0, other)
    started = time.time()
    print(f"resumed node {lagging}, killed node {other}", flush=True)

    caught_up = None
    turn = None
    survivor = f"http://{fleet.addr(0, lagging)}"
    worker = json.loads(subprocess.run([DRIVE, "agent", "worker"], capture_output=True,
                                       text=True, check=True).stdout)
    while time.time() - started < DEADLINE_SECS:
        if caught_up is None:
            lead, lag = applied(leader), applied(lagging)
            if lead is not None and lag is not None and lag >= lead - 1 and lag > paused_at:
                caught_up = round(time.time() - started, 1)
        if caught_up is not None and turn is None:
            try:
                session = call("POST", f"{survivor}/v1/sessions", {"agent": worker})["id"]
                call("POST", f"{survivor}/v1/sessions/{session}/messages", {"text": "go"})
                view = call("GET", f"{survivor}/v1/sessions/{session}/wait?turns=1&timeout_ms=30000",
                            timeout=40)
                if view.get("turns_completed") == 1:
                    turn = round(time.time() - started, 1)
                    break
            except Exception as error:
                print(f"turn not yet: {error}", flush=True)
        time.sleep(1)

    stats = {n: fleet.stats(0, n) for n in (leader, lagging) if applied(n) is not None}
    result = {
        "sessions": a.sessions,
        "entries_behind": behind,
        "caught_up_secs": caught_up,
        "turn_completed_secs": turn,
        "deadline_secs": DEADLINE_SECS,
        "leaders": {n: s.get("leader") for n, s in stats.items()},
    }
    result["ok"] = caught_up is not None and turn is not None
    print(json.dumps(result, indent=2))
    sys.exit(0 if result["ok"] else 1)


def cleanup():
    # A stopped process ignores SIGTERM until it runs again: wake every
    # node first, or a paused one keeps its port for the next run.
    for node in (1, 2, 3):
        try:
            os.kill(pid_of(node), signal.SIGCONT)
        except Exception:
            pass
    subprocess.run([sys.executable, os.path.join(ROOT, "scripts/cell/fleet.py"), "down"])


if __name__ == "__main__":
    # Run the cleanup on `timeout` and Ctrl-C too.
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(124))
    try:
        main()
    finally:
        cleanup()
