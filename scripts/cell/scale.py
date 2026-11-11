#!/usr/bin/env python3
"""Throughput and latency at 1..N cells, with a simulated model.

  scale.py --cells 1,2 --sessions 10000 --turns 2 --agent worker

For each cell count: a fresh fleet, one load run spread over every gateway,
then turns/s, admit and turn latency, per-node RSS and the cells' WAL stats.
"""
import argparse, json, os, subprocess, sys, time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "scripts", "cell"))
import fleet  # noqa: E402

DRIVE = os.path.join(ROOT, "target", "release", "cell-drive")


def rss_mb(cell, node):
    try:
        pid = int(open(fleet.pidfile(cell, node)).read())
        for line in open(f"/proc/{pid}/status"):
            if line.startswith("VmRSS:"):
                return round(int(line.split()[1]) / 1024)
    except Exception:
        return None


def run(cells, a):
    subprocess.run([sys.executable, os.path.join(ROOT, "scripts/cell/fleet.py"), "down"])
    time.sleep(0.5)
    subprocess.run(["rm", "-rf", fleet.STATE])
    os.environ["CELL_WORK_MS"] = a.work_ms
    fleet.up(cells, 3, 7100)
    time.sleep(1)
    gateways = ",".join(f"http://{fleet.addr(c, n)}" for c in range(cells) for n in (1, 2, 3))
    out = subprocess.run(
        [DRIVE, "load", "--gateways", gateways, "--agent", a.agent, "--sessions", str(a.sessions),
         "--turns", str(a.turns), "--concurrency", str(a.concurrency)],
        capture_output=True, text=True)
    report = json.loads(out.stdout)
    result = {
        "cells": cells,
        "sessions": a.sessions,
        "turns": report["turns_completed"],
        "errors": report["errors"],
        "turns_per_sec": round(report["turns_per_sec"], 1),
        "create_secs": round(report["create_secs"], 2),
        "admit_p50_ms": report["admit"]["p50_ms"],
        "admit_p99_ms": report["admit"]["p99_ms"],
        "turn_p50_ms": report["turn"]["p50_ms"],
        "turn_p99_ms": report["turn"]["p99_ms"],
        # Flat across the tenths of each session's turns: no slowdown with length.
        "turn_p50_ms_by_tenth": report.get("turn_p50_ms_by_tenth"),
        "rss_mb": {f"c{c}n{n}": rss_mb(c, n) for c in range(cells) for n in (1, 2, 3)},
    }
    print(json.dumps(result), flush=True)
    return result


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--cells", default="1,2")
    p.add_argument("--sessions", type=int, default=10000)
    p.add_argument("--turns", type=int, default=2)
    p.add_argument("--concurrency", type=int, default=2000)
    p.add_argument("--agent", default="worker")
    p.add_argument("--work-ms", default="50")
    a = p.parse_args()
    results = [run(int(c), a) for c in a.cells.split(",")]
    subprocess.run([sys.executable, os.path.join(ROOT, "scripts/cell/fleet.py"), "down"])
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
