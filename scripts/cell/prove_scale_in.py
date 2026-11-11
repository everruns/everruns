#!/usr/bin/env python3
"""Scale in: drain a cell under long turns, retire it, shut its nodes down.

  prove_scale_in.py [--sessions 120] [--idle 300] [--turns 3] [--scale 0.6]

Starts two cells and runs long multi-step turns in both (the llmsim
`long-tool` scenario), plus idle sessions that finished their one turn.
Then, as an autoscaler would:

  1. POST /v1/cells/1/drain: new sessions stop going to cell 1, and the
     rebalancer moves its sessions, running turns included, to cell 0.
  2. Waits until cell 1 holds no sessions.
  3. DELETE /v1/cells/1: the directory retires it. Gateways stop routing to
     it and service discovery stops listing it.
  4. kill -9 every node of cell 1.

The long turns keep running throughout. Every session is then checked with
the chaos invariants through cell 0's gateways only: each accepted message
completed exactly once, sequences are dense, and no tool ran more times than
it has durable `tool.started` events (the ledger).
"""
import argparse, json, os, subprocess, sys, time, urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "scripts", "cell"))
import fleet  # noqa: E402

DRIVE = os.path.join(ROOT, "target", "release", "cell-drive")


def request(method, url, timeout=30):
    req = urllib.request.Request(url, method=method, data=b"" if method != "GET" else None)
    try:
        with urllib.request.urlopen(req, timeout=timeout) as r:
            return r.status, json.loads(r.read() or b"null")
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()


def cell_load(cell):
    for node in fleet.nodes_of(cell):
        try:
            return request("GET", f"http://{fleet.addr(cell, node)}/cell/load", timeout=3)[1]
        except Exception:
            pass
    return None


def metric_sum(name, match="", cell=0):
    total = 0.0
    for node in fleet.nodes_of(cell):
        try:
            with urllib.request.urlopen(f"http://{fleet.addr(cell, node)}/metrics", timeout=3) as r:
                text = r.read().decode()
        except Exception:
            continue
        for line in text.splitlines():
            if line.startswith(name) and match in line:
                total += float(line.rsplit(" ", 1)[1])
    return int(total)


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--sessions", type=int, default=120)
    p.add_argument("--idle", type=int, default=300)
    p.add_argument("--turns", type=int, default=3)
    p.add_argument("--scale", default="0.6", help="CELL_TOOL_SCALE: 1 = 25 s per bash call")
    a = p.parse_args()

    subprocess.run([sys.executable, os.path.join(ROOT, "scripts/cell/fleet.py"), "down"])
    time.sleep(0.5)
    subprocess.run(["rm", "-rf", fleet.STATE])
    os.makedirs(fleet.STATE, exist_ok=True)
    ledger = os.path.join(fleet.STATE, "ledger.txt")
    os.environ["CELL_LEDGER"] = ledger
    os.environ["CELL_TOOL_SCALE"] = a.scale
    fleet.up(2, 3, 7100)
    time.sleep(1)
    gateways = [f"http://{fleet.addr(0, n)}" for n in (1, 2, 3)]
    gw = gateways[0]

    def load(name, sessions, turns, agent, scenarios=""):
        out = os.path.join(fleet.STATE, f"{name}.json")
        args = [DRIVE, "load", "--gateways", ",".join(gateways), "--agent", agent,
                "--sessions", str(sessions), "--turns", str(turns),
                "--concurrency", str(sessions),
                "--sessions-file", os.path.join(fleet.STATE, f"{name}.txt"), "--persistent"]
        if scenarios:
            args += ["--scenarios", scenarios]
        return subprocess.Popen(args, stdout=open(out, "w")), out

    def where(name):
        counts = {}
        for session in open(os.path.join(fleet.STATE, f"{name}.txt")).read().split():
            cell = request("GET", f"{gw}/cell/directory/{session}")[1]
            counts[str(cell)] = counts.get(str(cell), 0) + 1
        return counts

    idle_load, idle_out = load("idle", a.idle, 1, "worker3")
    idle_load.wait()
    long_load, long_out = load("long", a.sessions, a.turns, "sim", "long-tool")
    started = time.time()
    time.sleep(8)
    before = {"cell 0": cell_load(0), "cell 1": cell_load(1)}
    print(f"before drain: {json.dumps(before)}", flush=True)

    t = time.time()
    status, body = request("POST", f"{gw}/v1/cells/1/drain")
    print(f"drain cell 1: {status} {body}", flush=True)
    late_load, late_out = load("late", 60, 1, "worker3")
    while True:
        l1 = cell_load(1)
        print(f"{round(time.time() - t):>4}s  cell 1: {l1['active']:>3} turns "
              f"{l1['sessions']:>4} sessions {l1['moving']:>3} moving", flush=True)
        if l1["sessions"] == 0:
            break
        if time.time() - t > 300:
            sys.exit("cell 1 never drained")
        time.sleep(2)
    drained_secs = round(time.time() - t, 1)

    early = request("DELETE", f"{gw}/v1/cells/0")
    # Retiring waits for the cell's directory shards to move out too.
    for _ in range(60):
        status, body = request("DELETE", f"{gw}/v1/cells/1")
        if status != 409:
            break
        time.sleep(1)
    print(f"retire cell 1: {status} {body}", flush=True)
    retired = status == 200
    time.sleep(1.5)
    targets = request("GET", f"{gw}/v1/metrics/targets")[1]
    # Turns parked in cell 1 were counted by its executors: read before
    # its nodes go.
    parked_in_1 = metric_sum("cell_turns_executed_total", 'outcome="parked"', cell=1)
    for node in list(fleet.nodes_of(1)):
        fleet.kill(1, node)
    print(f"killed cell 1's nodes at {round(time.time() - started)}s; long turns still running",
          flush=True)

    long_load.wait()
    late_load.wait()
    time.sleep(1)

    checks = {}
    for name, turns in (("long", a.turns), ("idle", 1), ("late", 1)):
        check = subprocess.run(
            [DRIVE, "check", "--gateways", ",".join(gateways),
             "--sessions-file", os.path.join(fleet.STATE, f"{name}.txt"),
             "--turns", str(turns), "--ledger", ledger], capture_output=True, text=True)
        checks[name] = json.loads(check.stdout) if check.stdout.strip() else {"ok": False, "stderr": check.stderr}

    long_report = json.load(open(long_out))
    summary = {
        "sessions": {"long": a.sessions, "idle": a.idle, "created_while_draining": 60},
        "cell_1_before_drain": before["cell 1"],
        "secs_to_drain_cell_1": drained_secs,
        "drain_moves": {
            "busy": metric_sum("cell_rebalance_moves_total", 'kind="drain"'),
            "idle": metric_sum("cell_rebalance_moves_total", 'kind="drain-idle"'),
        },
        "turns_parked_mid_turn": parked_in_1
        + metric_sum("cell_turns_executed_total", 'outcome="parked"'),
        "retire_cell_0_refused": early[0],
        "retired": retired,
        "scrape_targets_after_retire": sorted(t for g in targets for t in g["targets"]),
        "sessions_by_cell_at_end": {name: where(name) for name in ("long", "idle", "late")},
        "turns": long_report["turns_completed"],
        "turn_p50_s": round(long_report["turn"]["p50_ms"] / 1000, 1),
        "load_errors": long_report["errors"] + json.load(open(late_out))["errors"],
        "check": {k: {"ok": v.get("ok"), "failed_sessions": v.get("failed_sessions"),
                      "tool_executions": v.get("totals", {}).get("tool_executions"),
                      "tool_started": v.get("totals", {}).get("tool_started")}
                  for k, v in checks.items()},
    }
    print(json.dumps(summary, indent=2))
    ok = retired and all(c.get("ok") for c in checks.values()) and summary["load_errors"] == 0
    sys.exit(0 if ok else 1)


if __name__ == "__main__":
    main()
