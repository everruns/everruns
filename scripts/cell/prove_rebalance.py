#!/usr/bin/env python3
"""Balance long turns across cells, moving them mid-turn.

  prove_rebalance.py [--sessions 120] [--turns 3] [--scale 0.6] [--kills 6]

Starts one cell and runs long multi-step turns in it (the llmsim
`long-tool` scenario: two bash calls of 25 s each, times --scale). While they
run it adds a second cell. Nothing else is told to move: the rebalancer on
the directory leader sees one busy cell and one empty cell and moves sessions,
running turns included. A moved turn parks at its next step boundary and the
new cell resumes it from the log. A second load created after the new cell
joined shows load-aware placement.

Every session is then checked with the chaos invariants: each accepted
message completed exactly once, sequences are dense, and no tool ran more
times than it has durable `tool.started` events (the ledger).
"""
import argparse, json, os, random, subprocess, sys, threading, time, urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "scripts", "cell"))
import fleet  # noqa: E402

DRIVE = os.path.join(ROOT, "target", "release", "cell-drive")


def get(url, timeout=3):
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return r.read().decode()


def cell_load(cell):
    for node in fleet.nodes_of(cell):
        try:
            return json.loads(get(f"http://{fleet.addr(cell, node)}/cell/load"))
        except Exception:
            pass
    return None


def metric_sum(name, match=""):
    """Sum a counter over every node's /metrics."""
    total = 0.0
    for cell in fleet.cells():
        for node in fleet.nodes_of(cell):
            try:
                text = get(f"http://{fleet.addr(cell, node)}/metrics")
            except Exception:
                continue
            for line in text.splitlines():
                if line.startswith(name) and match in line:
                    total += float(line.rsplit(" ", 1)[1])
    return int(total)


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--sessions", type=int, default=120)
    p.add_argument("--turns", type=int, default=3)
    p.add_argument("--scale", default="0.6", help="CELL_TOOL_SCALE: 1 = 25 s per bash call")
    p.add_argument("--add-after", type=float, default=8.0)
    p.add_argument("--kills", type=int, default=0,
                   help="also kill -9 this many nodes (one at a time) while sessions move")
    a = p.parse_args()

    subprocess.run([sys.executable, os.path.join(ROOT, "scripts/cell/fleet.py"), "down"])
    time.sleep(0.5)
    subprocess.run(["rm", "-rf", fleet.STATE])
    os.makedirs(fleet.STATE, exist_ok=True)
    ledger = os.path.join(fleet.STATE, "ledger.txt")
    os.environ["CELL_LEDGER"] = ledger
    os.environ["CELL_TOOL_SCALE"] = a.scale
    fleet.up(1, 3, 7100)
    time.sleep(1)
    gateways = [f"http://{fleet.addr(0, n)}" for n in (1, 2, 3)]

    def load(name, sessions, turns, agent, scenarios):
        out = os.path.join(fleet.STATE, f"{name}.json")
        args = [DRIVE, "load", "--gateways", ",".join(gateways), "--agent", agent,
                "--sessions", str(sessions), "--turns", str(turns),
                "--concurrency", str(sessions),
                "--sessions-file", os.path.join(fleet.STATE, f"{name}.txt"), "--persistent"]
        if scenarios:
            args += ["--scenarios", scenarios]
        return subprocess.Popen(args, stdout=open(out, "w")), out

    long_load, long_out = load("long", a.sessions, a.turns, "sim", "long-tool")
    started = time.time()
    timeline = []
    stop = threading.Event()

    def sample():
        while not stop.is_set():
            row = {"t": round(time.time() - started)}
            for cell in fleet.cells():
                l = cell_load(cell)
                if l:
                    row[f"c{cell}"] = l
            timeline.append(row)
            cells = "  ".join(
                f"cell {k[1:]}: {v['active']:>3} turns {v['sessions']:>4} sessions {v['moving']:>2} moving"
                for k, v in row.items() if k != "t")
            print(f"{row['t']:>4}s  {cells}", flush=True)
            stop.wait(3)

    sampler = threading.Thread(target=sample)
    sampler.start()

    time.sleep(a.add_after)
    print(f"--- adding cell 1 at {round(time.time() - started)}s", flush=True)
    added_at = time.time()
    fleet.add_cell(1)
    while cell_load(1) is None:
        time.sleep(0.2)
    # Wait until gateways know the new cell, then create more sessions.
    while '"1"' not in get(f"{gateways[0]}/v1/cells", timeout=10):
        time.sleep(0.2)
    time.sleep(1.5)
    short_load, short_out = load("short", 60, 1, "worker3", "")

    kills = []
    for _ in range(a.kills):
        cell, node = random.randrange(2), random.choice((1, 2, 3))
        was_leader = fleet.leader(cell) == node
        fleet.kill(cell, node)
        kills.append(was_leader)
        print(f"kill -9 cell {cell} node {node} (leader={was_leader})", flush=True)
        time.sleep(random.uniform(1.0, 3.0))
        fleet.start(cell, node)
        time.sleep(random.uniform(2.0, 5.0))

    long_load.wait()
    short_load.wait()
    stop.set()
    sampler.join()
    fleet.wait_ready()
    time.sleep(1)

    all_gateways = ",".join(f"http://{fleet.addr(c, n)}" for c in fleet.cells() for n in fleet.nodes_of(c))
    checks = {}
    for name, turns in (("long", a.turns), ("short", 1)):
        check = subprocess.run(
            [DRIVE, "check", "--gateways", all_gateways,
             "--sessions-file", os.path.join(fleet.STATE, f"{name}.txt"),
             "--turns", str(turns), "--ledger", ledger], capture_output=True, text=True)
        checks[name] = json.loads(check.stdout) if check.stdout.strip() else {"ok": False, "stderr": check.stderr}

    def placed(name):
        counts = {}
        for session in open(os.path.join(fleet.STATE, f"{name}.txt")).read().split():
            cell = get(f"{gateways[0]}/cell/directory/{session}").strip()
            counts[cell] = counts.get(cell, 0) + 1
        return counts

    def first_balanced():
        for row in timeline:
            c0, c1 = row.get("c0"), row.get("c1")
            if c0 and c1 and row["t"] >= added_at - started and c1["active"] > 0 \
                    and abs(c0["active"] - c1["active"]) <= max(4, (c0["active"] + c1["active"]) // 5):
                return round(row["t"] - (added_at - started))
        return None

    long_report = json.load(open(long_out))
    short_report = json.load(open(short_out))
    peak = max((r for r in timeline if "c1" in r), key=lambda r: r["c1"]["active"], default=None)
    summary = {
        "sessions": a.sessions,
        "turns": long_report["turns_completed"],
        "turn_p50_s": round(long_report["turn"]["p50_ms"] / 1000, 1),
        "turn_p99_s": round(long_report["turn"]["p99_ms"] / 1000, 1),
        "rebalance_moves": {
            "busy": metric_sum("cell_rebalance_moves_total", 'kind="busy"'),
            "idle": metric_sum("cell_rebalance_moves_total", 'kind="idle"'),
        },
        "turns_parked_mid_turn": metric_sum("cell_turns_executed_total", 'outcome="parked"'),
        "secs_until_balanced_after_add": first_balanced(),
        "peak_running_turns_in_new_cell": peak and peak["c1"]["active"],
        "long_sessions_by_cell_at_end": placed("long"),
        "short_sessions_placed_by_cell": placed("short"),
        "kills": len(kills),
        "leader_kills": sum(kills),
        "load_errors": long_report["errors"] + short_report["errors"],
        "check": checks,
    }
    with open(os.path.join(fleet.STATE, "timeline.json"), "w") as f:
        json.dump(timeline, f)
    print(json.dumps(summary, indent=2))
    ok = all(c.get("ok") for c in checks.values()) and summary["load_errors"] == 0
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
