#!/usr/bin/env python3
"""The object store goes away under load: no acknowledged write is lost and
shipping catches up when it is back (75k verifier point U11).

  prove_store_outage.py [--sessions 1000] [--outage 600]

1. One shard (3 nodes, 2 executors) runs `--sessions` `worker` sessions
   through a first turn, with the work tool's ledger.
2. The object store becomes unreachable: its directory is set aside and a
   plain file takes its place, so every put and get fails (a mode change
   would not stop a root process). Each session then gets a second message;
   a refused one (shedding while the log cannot ship) is retried until the
   store is back and the shard takes it.
3. After `--outage` seconds the store comes back. Within
   `--max-catch-up-s` the leader's oldest unshipped entry must fall back
   under 15 s and, if the shard purged log it had not shipped, a snapshot
   must have reached the store to bridge the gap.
4. Every node of the shard is then lost (SIGKILL, data set aside) and the
   shard rebuilt from the store alone, as `drill.py` does. Every second
   message the shard acknowledged must still be there: `agentd-drive check`
   with the ledger (each message completed once, sequences dense).

Pass: the check holds, shipping caught up in time, and no session lost a
message the shard acknowledged. Writes .shard/store_outage.json.
"""
import argparse, concurrent.futures, json, os, shutil, signal, subprocess, sys, time
import urllib.error, urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import fleet  # noqa: E402

DRIVE = os.environ.get("AGENTD_DRIVE_BIN", os.path.join(
    ROOT, os.environ.get("AGENTD_BIN_DIR", os.path.join("target", "release")), "agentd-drive"))

p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
p.add_argument("--sessions", type=int, default=1000)
p.add_argument("--outage", type=int, default=600, help="seconds the store is gone (goal: 10 minutes)")
p.add_argument("--max-catch-up-s", type=float, default=120.0)
p.add_argument("--max-rebuild-s", type=float, default=600.0)
a = p.parse_args()


def get(url, timeout=10):
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return json.load(r)


def post(url, body, timeout=30):
    req = urllib.request.Request(url, data=json.dumps(body).encode(), method="POST",
                                 headers={"content-type": "application/json"})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return r.status


def unshipped_s():
    """The leader's oldest unshipped entry's age, seconds (0: none waiting)."""
    lead = fleet.leader(0)
    if lead is None:
        return None
    try:
        r = get(f"http://{fleet.addr(0, lead)}/shard/inspect", timeout=3)
    except Exception:
        return None
    return (r.get("unshipped_ms") or 0) / 1000


subprocess.run([sys.executable, os.path.join(ROOT, "scripts/agentd/fleet.py"), "down"])
time.sleep(0.5)
shutil.rmtree(fleet.STATE, ignore_errors=True)
os.makedirs(fleet.STATE)
ledger = os.path.join(fleet.STATE, "ledger.txt")
os.environ.update({"AGENTD_LEDGER": ledger, "AGENTD_WORK_MS": "50"})
os.environ.setdefault("AGENTD_MEMORY_BUDGET_MB", "512")
fleet.up(1, 3, 7100, 2)
fleet.wait_ready()
t0 = time.time()
gws = [f"http://{fleet.addr(0, n)}" for n in (1, 2, 3)]
sf = os.path.join(fleet.STATE, "sessions.txt")

# 1. First turns.
out = subprocess.run([DRIVE, "load", "--gateways", ",".join(gws), "--agent", "worker",
                      "--sessions", str(a.sessions), "--turns", "1", "--concurrency", "200",
                      "--sessions-file", sf, "--persistent"], capture_output=True, text=True)
load = json.loads(out.stdout or "{}")
ids = [l for l in open(sf).read().splitlines() if l]
print(json.dumps({"load": {k: load.get(k) for k in ("sessions", "turns_completed", "errors")}}), flush=True)

# 2. The store goes away; second messages arrive while it is gone.
store = os.path.join(fleet.STATE, "store")
os.rename(store, store + ".away")
open(store, "w").close()
outage_at = time.time()
print(f"t={round(outage_at - t0)}: store unreachable", flush=True)
acked, refused, failed = {}, {"n": 0}, {}
deadline = outage_at + a.outage + a.max_catch_up_s + 300


def second(i_sid):
    i, sid = i_sid
    while time.time() < deadline:
        gw = gws[i % len(gws)]
        try:
            post(f"{gw}/v1/sessions/{sid}/messages", {"text": "turn 2"})
            acked[sid] = round(time.time() - outage_at, 1)
            return
        except urllib.error.HTTPError as e:
            if e.code in (429, 503):
                refused["n"] += 1
            else:
                failed[sid] = e.code
                return
        except Exception:
            pass
        i += 1
        time.sleep(2)
    failed[sid] = "timeout"


pool = concurrent.futures.ThreadPoolExecutor(max_workers=64)
futures = [pool.submit(second, x) for x in enumerate(ids)]
timeline = []
while time.time() - outage_at < a.outage:
    timeline.append({"t": round(time.time() - outage_at), "unshipped_s": unshipped_s(), "acked": len(acked)})
    time.sleep(10)
acked_during_outage = len(acked)

# 3. The store comes back; shipping must catch up.
os.remove(store)
os.rename(store + ".away", store)
back_at = time.time()
print(f"t={round(back_at - t0)}: store back, {acked_during_outage} second messages acknowledged while it was gone", flush=True)
caught_up_s = None


def gap_covered():
    """The store can rebuild the shard: either the log shipped without a
    purge gap, or a snapshot reached the store to bridge it (`sm.rs`)."""
    logs = os.path.join(fleet.STATE, "logs")
    gap = any("purged into a snapshot before it shipped" in open(os.path.join(logs, n), errors="replace").read()
              for n in os.listdir(logs))
    snaps = os.path.join(store, "shards", "0", "snapshots")
    return not gap or (os.path.isdir(snaps) and any(f.endswith(".snap") for f in os.listdir(snaps)))


while time.time() - back_at < a.max_catch_up_s * 3:
    u = unshipped_s()
    timeline.append({"t": round(time.time() - outage_at), "unshipped_s": u, "acked": len(acked)})
    if u is not None and u <= 15 and caught_up_s is None and gap_covered():
        caught_up_s = round(time.time() - back_at, 1)
    if caught_up_s is not None and all(f.done() for f in futures):
        break
    time.sleep(2)
for f in futures:
    f.result()
# Let every acknowledged turn finish and ship before the shard is lost.
check_all = subprocess.run([DRIVE, "check", "--gateways", ",".join(gws), "--sessions-file", sf,
                            "--turns", "2", "--ledger", ledger], capture_output=True, text=True)
before = json.loads(check_all.stdout) if check_all.stdout.strip() else {"ok": False}
for _ in range(60):
    u = unshipped_s()
    if u is not None and u == 0:
        break
    time.sleep(2)

# 4. Lose every node; rebuild from the store alone.
for n in (1, 2, 3):
    fleet.kill(0, n, signal.SIGKILL)
time.sleep(1)
for n in (1, 2, 3):
    data = os.path.join(fleet.STATE, "data", f"c0n{n}")
    if os.path.isdir(data):
        os.rename(data, data + ".lost")
started = time.time()
rebuilt = subprocess.run([fleet.binary(), "restore", "--topology", fleet.TOPO, "--shard", "0",
                          "--store", "file://" + store]
                         + [f"--node={n}={os.path.join(fleet.STATE, 'data', f'c0n{n}')}" for n in (1, 2, 3)],
                         capture_output=True, text=True)
rebuild_s = round(time.time() - started, 1)
if rebuilt.returncode != 0:
    sys.exit(f"restore failed: {rebuilt.stderr[-2000:]}")
for n in (1, 2, 3):
    fleet.start(0, n)
while fleet.leader(0) is None:
    if time.time() - started > a.max_rebuild_s:
        sys.exit("the rebuilt shard elected no leader")
    time.sleep(0.2)
chk = subprocess.run([DRIVE, "check", "--gateways", ",".join(gws), "--sessions-file", sf,
                      "--turns", "2", "--ledger", ledger], capture_output=True, text=True)
after = json.loads(chk.stdout) if chk.stdout.strip() else {"ok": False, "err": chk.stderr[-2000:]}

res = {
    "sessions": len(ids), "outage_s": a.outage,
    "load": {k: load.get(k) for k in ("turns_completed", "errors")},
    "acked": len(acked), "acked_during_outage": acked_during_outage,
    "refused_retried": refused["n"], "failed": len(failed), "first_failed": list(failed.items())[:5],
    "caught_up_s": caught_up_s, "rebuild_s": rebuild_s,
    "check_before_loss": {k: before.get(k) for k in ("ok", "failed_sessions")},
    "check": {k: after.get(k) for k in ("ok", "failed_sessions", "totals", "failures", "err")},
    "timeline": timeline, "wall_s": round(time.time() - t0),
}
res["ok"] = bool(after.get("ok")) and not failed and caught_up_s is not None \
    and caught_up_s <= a.max_catch_up_s and load.get("errors") == 0
res["errors"] = len(failed)
json.dump(res, open(os.path.join(fleet.STATE, "store_outage.json"), "w"), indent=1)
subprocess.run([sys.executable, os.path.join(ROOT, "scripts/agentd/fleet.py"), "down"])
print(json.dumps({k: v for k, v in res.items() if k != "timeline"}, indent=1))
sys.exit(0 if res["ok"] else 1)
