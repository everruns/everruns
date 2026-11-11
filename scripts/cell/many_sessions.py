#!/usr/bin/env python3
"""Many sessions on a local fleet: create them all, run their turns, check every one.

  many_sessions.py                          # 75 000 sessions x 2 turns, 2 cells
  many_sessions.py --sessions 20000 --turns 3 --cells 3

Starts a fresh fleet (3 nodes per cell, 128 MB memory budget per node), drives
`--sessions` `worker` sessions (`--concurrency` at once) through `--turns`
turns each, samples every node's RSS every 20 s, then checks every session:
each message completed exactly once, sequences dense, no tool ran more times
than it has durable `tool.started` events. Writes .cell/many_sessions.json.
"""
import argparse, json, os, subprocess, sys, threading, time, urllib.request
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "scripts", "cell"))
import fleet
DRIVE = os.path.join(ROOT, "target/release/cell-drive")
p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
p.add_argument("--cells", type=int, default=2)
p.add_argument("--sessions", type=int, default=75000)
p.add_argument("--turns", type=int, default=2)
p.add_argument("--concurrency", type=int, default=3000)
a = p.parse_args()
CELLS, N, TURNS, CONC = a.cells, a.sessions, a.turns, a.concurrency
subprocess.run([sys.executable, os.path.join(ROOT, "scripts/cell/fleet.py"), "down"]); time.sleep(0.5)
subprocess.run(["rm", "-rf", fleet.STATE]); os.makedirs(fleet.STATE)
ledger = os.path.join(fleet.STATE, "ledger.txt")
os.environ.update({"CELL_LEDGER": ledger, "CELL_MEMORY_BUDGET_MB": "128", "CELL_WORK_MS": "50"})
fleet.up(CELLS, 3, 7100); time.sleep(1)
gws = ",".join(f"http://{fleet.addr(c, n)}" for c in range(CELLS) for n in (1, 2, 3))
sf = os.path.join(fleet.STATE, "sessions.txt")
def metric(c, n, name):
    try:
        for l in urllib.request.urlopen(f"http://{fleet.addr(c,n)}/metrics", timeout=3).read().decode().splitlines():
            if l.startswith(name): return float(l.rsplit(" ", 1)[1])
    except Exception: return None
samples, stop = [], threading.Event()
t0 = time.time()
def sample():
    while not stop.wait(20):
        rss = {f"c{c}n{n}": round((metric(c, n, "cell_rss_bytes") or 0) / 2**20) for c in range(CELLS) for n in (1, 2, 3)}
        sess = [json.loads(urllib.request.urlopen(f"http://{fleet.addr(c,1)}/cell/load", timeout=3).read()) for c in range(CELLS)]
        row = {"t": round(time.time() - t0), "rss": rss, "sessions": [s["sessions"] for s in sess], "active": [s["active"] for s in sess]}
        samples.append(row); print(json.dumps(row), flush=True)
threading.Thread(target=sample, daemon=True).start()
out = subprocess.run([DRIVE, "load", "--gateways", gws, "--agent", "worker", "--sessions", str(N), "--turns", str(TURNS),
                      "--concurrency", str(CONC), "--sessions-file", sf, "--persistent"], capture_output=True, text=True)
stop.set()
rep = json.loads(out.stdout); rep.pop("cells", None)
time.sleep(30)  # let unloading settle
rss_end = {f"c{c}n{n}": round((metric(c, n, "cell_rss_bytes") or 0) / 2**20) for c in range(CELLS) for n in (1, 2, 3)}
chk = subprocess.run([DRIVE, "check", "--gateways", gws, "--sessions-file", sf, "--turns", str(TURNS), "--ledger", ledger], capture_output=True, text=True)
c = json.loads(chk.stdout) if chk.stdout.strip() else {"ok": False, "err": chk.stderr[-2000:]}
res = {"sessions": rep["sessions"], "turns": rep["turns_completed"], "errors": rep["errors"], "turns_per_sec": rep["turns_per_sec"],
       "turn": rep["turn"], "admit": rep.get("admit"), "rss_peak": max(max(r["rss"].values()) for r in samples) if samples else None,
       "rss_end": rss_end, "wall": round(time.time() - t0), "check": {k: c.get(k) for k in ("ok", "failed_sessions", "totals", "err")}}
json.dump({"result": res, "samples": samples}, open(os.path.join(fleet.STATE, "many_sessions.json"), "w"), indent=1)
print(json.dumps(res, indent=1))
subprocess.run([sys.executable, os.path.join(ROOT, "scripts/cell/fleet.py"), "down"])
sys.exit(0 if c.get("ok") and rep["errors"] == 0 else 1)
