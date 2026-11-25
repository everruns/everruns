#!/usr/bin/env python3
"""Disaster recovery drill: lose every node of a shard under load, rebuild it
from the object store, and check that nothing ran twice and every session
carries on (data plane v2 step 9, design.md "Disaster recovery").

  drill.py                                  # 75 000 sessions, 2 shards
  drill.py --sessions 2000 --inflight 300   # the weekly CI size

1. A fresh fleet (3 nodes per shard) runs `--sessions` `worker` sessions
   through one turn each. The shards' shipping lag and ship-first cost come
   from their leaders' /metrics after this load.
2. `--inflight` sessions get a second message, and while those turns run
   every node of shard 0 (the directory shard, about half the sessions) is
   killed with SIGKILL, and its data directories set aside as lost.
3. `agentd-testnode restore` rebuilds shard 0 from the store into empty data
   directories (timed), its nodes start again, and the drill times how long
   until shard 0 has a leader.
4. A second message the rebuild lost (a plain commit inside the shipping
   window: up to 10 s before the kill, never more than 30 s) is sent again;
   the drill counts these. Session creates and the starts of tools not safe
   to repeat are never lost: they reach the store before they are answered
   (guarantees.md, ship-first writes). Then every session must finish: `agentd-drive check` with the work
   tool's ledger (each message completed once, sequences dense, no tool ran
   more often than it has a durable `tool.started`).

Writes .shard/drill.json. Fails on a failed check, a rebuild slower than
`--max-rebuild-s`, or a shipping lag p99 over `--max-ship-lag-s`.
"""
import argparse, concurrent.futures, json, os, shutil, signal, subprocess, sys, time
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "scripts", "shard"))
import fleet  # noqa: E402

DRIVE = os.path.join(ROOT, "target/release/agentd-drive")

p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
p.add_argument("--shards", type=int, default=2)
p.add_argument("--sessions", type=int, default=75000)
p.add_argument("--inflight", type=int, default=3000)
p.add_argument("--concurrency", type=int, default=1000)
p.add_argument("--kill-after-s", type=float, default=3.0)
p.add_argument("--max-rebuild-s", type=float, default=600.0)
# The shipped log's lag: an object at most every 10 s (`ship::SHIP_EVERY`)
# plus a tick; the 75k goal's F3 bound is a p99 of 15 s.
p.add_argument("--max-ship-lag-s", type=float, default=15.0)
a = p.parse_args()


def get(url, timeout=10):
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return json.load(r)


def post(url, body, timeout=10):
    req = urllib.request.Request(url, data=json.dumps(body).encode(), method="POST",
                                 headers={"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return json.load(r)


def histogram(shard, name):
    """Cumulative bucket counts of `name` summed over a shard's nodes."""
    buckets = {}
    for node in fleet.nodes_of(shard):
        try:
            text = urllib.request.urlopen(f"http://{fleet.addr(shard, node)}/metrics", timeout=5).read().decode()
        except Exception:
            continue
        for line in text.splitlines():
            if not line.startswith(name + "_bucket"):
                continue
            le = line.split('le="', 1)[1].split('"', 1)[0]
            bound = float("inf") if le == "+Inf" else float(le)
            buckets[bound] = buckets.get(bound, 0) + float(line.rsplit(" ", 1)[1])
    return sorted(buckets.items())


def quantile(buckets, q):
    """The upper bound of the bucket holding quantile `q` (None: no data)."""
    if not buckets or buckets[-1][1] == 0:
        return None
    total = buckets[-1][1]
    for bound, count in buckets:
        if count >= q * total:
            return bound
    return buckets[-1][0]


def summary(name):
    merged = {}
    for c in range(a.shards):
        for bound, count in histogram(c, name):
            merged[bound] = merged.get(bound, 0) + count
    b = sorted(merged.items())
    return {"count": int(b[-1][1]) if b else 0, "p50": quantile(b, 0.5), "p99": quantile(b, 0.99)}


subprocess.run([sys.executable, os.path.join(ROOT, "scripts/agentd/fleet.py"), "down"])
time.sleep(0.5)
shutil.rmtree(fleet.STATE, ignore_errors=True)
os.makedirs(fleet.STATE)
ledger = os.path.join(fleet.STATE, "ledger.txt")
os.environ.update({"AGENTD_LEDGER": ledger, "AGENTD_MEMORY_BUDGET_MB": "128", "AGENTD_WORK_MS": "50"})
fleet.up(a.shards, 3, 7100)
time.sleep(1)
t0 = time.time()
all_gws = [f"http://{fleet.addr(c, n)}" for c in range(a.shards) for n in (1, 2, 3)]
# Second messages go through the shards that stay up.
safe_gws = [f"http://{fleet.addr(c, n)}" for c in range(1, a.shards) for n in (1, 2, 3)] or all_gws
sf = os.path.join(fleet.STATE, "sessions.txt")

# 1. Load.
out = subprocess.run([DRIVE, "load", "--gateways", ",".join(all_gws), "--agent", "worker",
                      "--sessions", str(a.sessions), "--turns", "1", "--concurrency", str(a.concurrency),
                      "--sessions-file", sf, "--persistent"], capture_output=True, text=True)
load = json.loads(out.stdout)
load.pop("shards", None)
ship_lag = summary("agentd_shard_ship_lag_seconds")
ship_first = summary("agentd_shard_ship_first_duration_seconds")
print(json.dumps({"load": {k: load.get(k) for k in ("sessions", "turns_completed", "errors", "turns_per_sec")},
                  "ship_lag": ship_lag, "ship_first": ship_first}), flush=True)
ids = [l for l in open(sf).read().splitlines() if l]
second = ids[: a.inflight]

# 2. Second messages in flight, then lose shard 0.
sent = {}


def say(i_id):
    i, sid = i_id
    gw = safe_gws[i % len(safe_gws)]
    try:
        post(f"{gw}/v1/sessions/{sid}/messages", {"text": "turn 2"}, timeout=30)
        sent[sid] = True
    except Exception:
        sent[sid] = False  # sent again below if the rebuild lost it


pool = concurrent.futures.ThreadPoolExecutor(max_workers=64)
futures = [pool.submit(say, x) for x in enumerate(second)]
time.sleep(a.kill_after_s)
killed_at = time.time()
for n in (1, 2, 3):
    fleet.kill(0, n, signal.SIGKILL)
time.sleep(1)
for n in (1, 2, 3):
    data = os.path.join(fleet.STATE, "data", f"c0n{n}")
    if os.path.isdir(data):
        os.rename(data, data + ".lost")

# 3. Rebuild from the store, start, wait for a leader.
store = "file://" + os.path.join(fleet.STATE, "store")
started = time.time()
rebuilt = subprocess.run([fleet.binary(), "restore", "--topology", fleet.TOPO, "--shard", "0", "--store", store]
                         + [f"--node={n}={os.path.join(fleet.STATE, 'data', f'c0n{n}')}" for n in (1, 2, 3)],
                         capture_output=True, text=True)
rebuild_s = time.time() - started
if rebuilt.returncode != 0:
    sys.exit(f"restore failed: {rebuilt.stderr[-2000:]}")
report = json.loads(rebuilt.stdout)
for n in (1, 2, 3):
    fleet.start(0, n)
while fleet.leader(0) is None:
    if time.time() - started > a.max_rebuild_s:
        sys.exit("shard 0 elected no leader after the rebuild")
    time.sleep(0.2)
back_s = time.time() - killed_at
for f in futures:
    f.result()

# 4. Send again what the rebuild lost, then everything must finish.
lost = 0
for i, sid in enumerate(second):
    view = None
    for _ in range(100):
        try:
            view = get(f"{all_gws[i % len(all_gws)]}/v1/sessions/{sid}/events", timeout=30)
            break
        except Exception:
            time.sleep(0.5)
    inputs = sum(1 for e in (view or {}).get("data", []) if e.get("type") == "input.message")
    if inputs < 2:
        lost += 1
        for _ in range(100):
            try:
                post(f"{all_gws[i % len(all_gws)]}/v1/sessions/{sid}/messages", {"text": "turn 2"}, timeout=30)
                break
            except Exception:
                time.sleep(0.5)
sf2, sf1 = os.path.join(fleet.STATE, "second.txt"), os.path.join(fleet.STATE, "first.txt")
open(sf2, "w").write("\n".join(second) + "\n")
open(sf1, "w").write("\n".join(ids[a.inflight:]) + "\n")
checks = {}
for name, path, turns in (("second", sf2, 2), ("first", sf1, 1)):
    chk = subprocess.run([DRIVE, "check", "--gateways", ",".join(all_gws), "--sessions-file", path,
                          "--turns", str(turns), "--ledger", ledger], capture_output=True, text=True)
    c = json.loads(chk.stdout) if chk.stdout.strip() else {"ok": False, "err": chk.stderr[-2000:]}
    checks[name] = {k: c.get(k) for k in ("ok", "failed_sessions", "totals", "failures", "err")}

res = {
    "sessions": a.sessions, "shards": a.shards, "inflight": a.inflight,
    "load": {k: load.get(k) for k in ("turns_completed", "errors", "turns_per_sec", "turn")},
    "ship_lag_s": ship_lag, "ship_first_s": ship_first,
    "rebuilt": report, "rebuild_s": round(rebuild_s, 1), "kill_to_leader_s": round(back_s, 1),
    "second_messages_lost": lost, "check": checks, "wall_s": round(time.time() - t0),
}
json.dump(res, open(os.path.join(fleet.STATE, "drill.json"), "w"), indent=1)
print(json.dumps(res, indent=1))
subprocess.run([sys.executable, os.path.join(ROOT, "scripts/agentd/fleet.py"), "down"])
ok = all(c.get("ok") for c in checks.values()) and load.get("errors") == 0
ok &= rebuild_s <= a.max_rebuild_s
ok &= ship_lag["p99"] is None or ship_lag["p99"] <= a.max_ship_lag_s
sys.exit(0 if ok else 1)
