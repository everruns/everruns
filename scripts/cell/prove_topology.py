#!/usr/bin/env python3
"""Change the fleet's shape while it runs, under load.

  prove_topology.py

Starts one cell from the seed topology and runs multi-step turns. While they
run it adds a fourth node to cell 0, starts a new cell 1 (which registers
itself with the directory), moves sessions into the new cell, then removes
and kills one of cell 0's original nodes. A second load created after cell 1
joined must land partly in it, through gateways that only ever read the seed
file. Both loads are checked with the chaos invariants, and the seed file is
checked to be unchanged.
"""
import hashlib, json, os, subprocess, sys, threading, time, urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "scripts", "cell"))
import fleet  # noqa: E402

DRIVE = os.path.join(ROOT, "target", "release", "cell-drive")


def get(url, timeout=10):
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return json.load(r)


def request(method, url, body=None, timeout=120):
    data = json.dumps(body).encode() if body is not None else None
    req = urllib.request.Request(url, data=data, method=method,
                                 headers={"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.load(r)


def sessions_per_cell(gateway):
    out = {}
    for cell, nodes in get(f"{gateway}/v1/cells").items():
        counts = [n["stats"]["sessions"] for n in nodes if n["stats"]]
        out[cell] = max(counts) if counts else None
    return out


def main():
    subprocess.run([sys.executable, os.path.join(ROOT, "scripts/cell/fleet.py"), "down"])
    time.sleep(0.5)
    subprocess.run(["rm", "-rf", fleet.STATE])
    os.makedirs(fleet.STATE, exist_ok=True)
    ledger = os.path.join(fleet.STATE, "ledger.txt")
    os.environ["CELL_LEDGER"] = ledger
    os.environ["CELL_WORK_MS"] = "200"
    fleet.up(1, 3, 7100)
    seed_hash = hashlib.sha256(open(fleet.TOPO, "rb").read()).hexdigest()
    time.sleep(1)
    gw = lambda node: f"http://{fleet.addr(0, node)}"  # noqa: E731
    steps = []

    def load(name, sessions, turns, gateways):
        out = os.path.join(fleet.STATE, f"{name}.json")
        return subprocess.Popen(
            [DRIVE, "load", "--gateways", ",".join(gateways), "--agent", "worker3",
             "--sessions", str(sessions), "--turns", str(turns), "--concurrency", str(sessions),
             "--sessions-file", os.path.join(fleet.STATE, f"{name}.txt"), "--persistent"],
            stdout=open(out, "w")), out

    a, a_out = load("a", 300, 4, [gw(1), gw(2), gw(3)])
    time.sleep(2)

    t = time.time()
    joined = fleet.join(0, 4)
    steps.append({"step": "join c0n4", "secs": round(time.time() - t, 2),
                  "members": sorted(joined["members"])})

    t = time.time()
    fleet.add_cell(1)
    while "1" not in get(f"{gw(2)}/v1/cells"):
        if time.time() - t > 60:
            sys.exit("gateways never saw cell 1")
        time.sleep(0.2)
    steps.append({"step": "add cell 1", "secs_until_gateways_see_it": round(time.time() - t, 2)})

    ids = [l for l in open(os.path.join(fleet.STATE, "a.txt")).read().split("\n") if l]
    moved = 0
    t = time.time()
    for session in ids[:30]:
        r = request("POST", f"{gw(2)}/v1/sessions/{session}/move", {"cell": 1})
        moved += bool(r.get("moved"))
    steps.append({"step": "move 30 sessions to cell 1", "moved": moved,
                  "secs": round(time.time() - t, 2)})

    t = time.time()
    was_leader = fleet.leader(0) == 1
    removed = request("DELETE", f"{gw(2)}/v1/cells/0/nodes/1")
    fleet.kill(0, 1)
    steps.append({"step": "remove and kill c0n1", "was_leader": was_leader,
                  "members": sorted(removed["members"]), "secs": round(time.time() - t, 2)})

    b, b_out = load("b", 200, 2, [gw(2), gw(4)])
    b.wait()
    a.wait()
    spread = sessions_per_cell(gw(2))

    def check(name, turns):
        out = subprocess.run(
            [DRIVE, "check", "--gateways", f"{gw(2)},{gw(4)}", "--sessions-file",
             os.path.join(fleet.STATE, f"{name}.txt"), "--turns", str(turns), "--ledger", ledger],
            capture_output=True, text=True)
        return json.loads(out.stdout)

    # The ledger holds both loads' tool runs; each check counts its own sessions.
    report = {
        "steps": steps,
        "sessions_per_cell": spread,
        "load_a": {k: json.load(open(a_out))[k] for k in ("turns_completed", "errors")},
        "load_b": {k: json.load(open(b_out))[k] for k in ("turns_completed", "errors")},
        "check_a": check("a", 4),
        "check_b": check("b", 2),
        "seed_file_unchanged": hashlib.sha256(open(fleet.TOPO, "rb").read()).hexdigest() == seed_hash,
    }
    for k in ("check_a", "check_b"):
        report[k] = {"ok": report[k]["ok"], "failed_sessions": report[k]["failed_sessions"],
                     "failures": report[k]["failures"][:3]}
    print(json.dumps(report, indent=2))
    subprocess.run([sys.executable, os.path.join(ROOT, "scripts/cell/fleet.py"), "down"])
    sys.exit(0 if report["check_a"]["ok"] and report["check_b"]["ok"] else 1)


if __name__ == "__main__":
    main()
