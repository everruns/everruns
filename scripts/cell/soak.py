#!/usr/bin/env python3
"""A long, production-shaped run on a local fleet: steady arrivals, think
time between messages, real tool timings, optional node kills.

  soak.py                                            # 30 min, 2 cells, 10 new sessions/s
  soak.py --minutes 60 --rate 20 --think 20 --turns 5 --kill-every 300
  soak.py --minutes 5 --rate 5                       # a quick look

Sessions arrive at `--rate` per second (Poisson) for `--minutes`, each runs
`--turns` turns of the llmsim scenario mix with an exponential pause of
`--think` seconds (mean) between them, so the fleet holds a steady
population of active sessions, most of them idle between messages, like
users. Every 30 s it samples each cell (running turns, sessions, unloaded
sessions) and each node's RSS. With `--kill-every`, a random node is killed
with SIGKILL and restarted at that interval.

At the end every session is checked: each message completed exactly once,
sequences are dense, and no tool ran more times than it has durable
`tool.started` events. The timeline goes to .cell/soak.json.
"""
import argparse, json, os, random, subprocess, sys, threading, time, urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "scripts", "cell"))
import fleet  # noqa: E402

DRIVE = os.path.join(ROOT, "target", "release", "cell-drive")


def get(url, timeout=3):
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return r.read().decode()


def rss_mb(cell, node):
    try:
        for line in get(f"http://{fleet.addr(cell, node)}/metrics").splitlines():
            if line.startswith("cell_rss_bytes"):
                return round(float(line.rsplit(" ", 1)[1]) / 2**20)
    except Exception:
        return None


def cell_sample(cell):
    for node in fleet.nodes_of(cell):
        try:
            load = json.loads(get(f"http://{fleet.addr(cell, node)}/cell/load"))
            stats = json.loads(get(f"http://{fleet.addr(cell, node)}/cell/stats"))
            return {"running": load["active"], "queued": load["queued"],
                    "sessions": load["sessions"], "unloaded": stats.get("unloaded_sessions")}
        except Exception:
            pass
    return None


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--cells", type=int, default=2)
    p.add_argument("--minutes", type=float, default=30)
    p.add_argument("--rate", type=float, default=10, help="new sessions per second")
    p.add_argument("--turns", type=int, default=5, help="turns per session")
    p.add_argument("--think", type=float, default=15, help="mean seconds between a session's turns")
    p.add_argument("--scenarios", default="mix")
    p.add_argument("--scale", default="1", help="CELL_TOOL_SCALE (1 = real tool durations)")
    p.add_argument("--memory-mb", default="128", help="memory budget per node")
    p.add_argument("--kill-every", type=float, default=0, help="seconds between kills (0: none)")
    a = p.parse_args()

    subprocess.run([sys.executable, os.path.join(ROOT, "scripts/cell/fleet.py"), "down"])
    time.sleep(0.5)
    subprocess.run(["rm", "-rf", fleet.STATE])
    os.makedirs(fleet.STATE, exist_ok=True)
    ledger = os.path.join(fleet.STATE, "ledger.txt")
    os.environ.update({"CELL_LEDGER": ledger, "CELL_TOOL_SCALE": a.scale,
                       "CELL_MEMORY_BUDGET_MB": a.memory_mb})
    fleet.up(a.cells, 3, 7100)
    time.sleep(1)

    gateways = ",".join(f"http://{fleet.addr(c, n)}" for c in range(a.cells) for n in (1, 2, 3))
    sessions_file = os.path.join(fleet.STATE, "sessions.txt")
    out = os.path.join(fleet.STATE, "soak-load.json")
    print(f"soak: {a.minutes:g} min, {a.rate:g} new sessions/s, {a.turns} turns each, "
          f"think {a.think:g} s, tool scale {a.scale}, {a.cells} cells", flush=True)
    load = subprocess.Popen(
        [DRIVE, "load", "--gateways", gateways, "--agent", "sim", "--scenarios", a.scenarios,
         "--rate", str(a.rate), "--duration", str(a.minutes * 60), "--sessions", "0",
         "--turns", str(a.turns), "--think", str(a.think), "--sessions-file", sessions_file,
         "--persistent"], stdout=open(out, "w"))
    started = time.time()
    timeline, kills = [], []
    stop = threading.Event()

    def sample():
        while not stop.wait(30):
            row = {"t": round(time.time() - started),
                   "cells": {c: cell_sample(c) for c in range(a.cells)},
                   "rss_mb": {f"c{c}n{n}": rss_mb(c, n) for c in range(a.cells) for n in (1, 2, 3)}}
            timeline.append(row)
            cells = "  ".join(f"cell {c}: {s['running']:>4} running {s['sessions']:>6} sessions "
                              f"({s['unloaded'] or 0} unloaded)"
                              for c, s in row["cells"].items() if s)
            rss = [v for v in row["rss_mb"].values() if v]
            print(f"{row['t']:>6}s  {cells}  RSS max {max(rss) if rss else '?'} MB", flush=True)

    def chaos():
        while not stop.wait(a.kill_every):
            cell, node = random.randrange(a.cells), random.choice((1, 2, 3))
            was_leader = fleet.leader(cell) == node
            fleet.kill(cell, node)
            kills.append({"t": round(time.time() - started), "cell": cell, "node": node,
                          "leader": was_leader})
            print(f"kill -9 cell {cell} node {node} (leader={was_leader})", flush=True)
            time.sleep(random.uniform(2, 5))
            fleet.start(cell, node)

    threads = [threading.Thread(target=sample, daemon=True)]
    if a.kill_every > 0:
        threads.append(threading.Thread(target=chaos, daemon=True))
    for t in threads:
        t.start()
    load.wait()
    stop.set()
    fleet.wait_ready()
    time.sleep(1)

    print("load done; checking every session", flush=True)
    check = subprocess.run([DRIVE, "check", "--gateways", gateways, "--sessions-file", sessions_file,
                            "--turns", str(a.turns), "--ledger", ledger], capture_output=True, text=True)
    result = json.loads(check.stdout) if check.stdout.strip() else {"ok": False, "stderr": check.stderr}
    report = json.load(open(out))
    report.pop("cells", None)
    peak = lambda key: max((sum((s or {}).get(key) or 0 for s in r["cells"].values()) for r in timeline), default=0)  # noqa: E731
    rss = [v for r in timeline for v in r["rss_mb"].values() if v]
    summary = {
        "minutes": a.minutes, "rate": a.rate, "turns_per_session": a.turns, "think_secs": a.think,
        "tool_scale": a.scale, "cells": a.cells,
        "sessions": report["sessions"], "turns": report["turns_completed"], "errors": report["errors"],
        "turns_per_sec": round(report["turns_per_sec"], 1),
        "turn": report["turn"], "admit": report["admit"], "scenarios": report["scenarios"],
        "peak_running_turns": peak("running"), "peak_sessions": peak("sessions"),
        "peak_unloaded": peak("unloaded"),
        "rss_mb": {"max": max(rss, default=None),
                   "last": {k: v for k, v in timeline[-1]["rss_mb"].items()} if timeline else None},
        "kills": len(kills), "leader_kills": sum(k["leader"] for k in kills),
        "check": {k: result.get(k) for k in ("ok", "failed_sessions", "totals", "failures")},
    }
    with open(os.path.join(fleet.STATE, "soak.json"), "w") as f:
        json.dump({"summary": summary, "timeline": timeline, "kills": kills}, f, indent=1)
    print(json.dumps(summary, indent=2))
    sys.exit(0 if result.get("ok") and report["errors"] == 0 else 1)


if __name__ == "__main__":
    main()
