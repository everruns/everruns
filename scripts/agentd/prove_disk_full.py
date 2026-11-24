#!/usr/bin/env python3
"""A shard node's disk fills up under load: the node stops taking writes, the
shard carries on, and the node comes back whole once there is room (75k
verifier point U12).

  prove_disk_full.py [--rate 10] [--secs 240] [--node leader|follower]

Needs root and mount rights (a privileged container): the node's data
directory moves onto a small tmpfs, which the proof then fills.

1. One shard (3 nodes, 2 executors). The chosen node (`--node`, default a
   follower) is stopped, its data directory copied onto a `--tmpfs-mb`
   tmpfs, and the node started again on it.
2. `worker` sessions arrive at `--rate` a second for `--secs`, through the
   other nodes' gateways, with the work tool's ledger. `--fill-at` seconds in, a
   file fills the tmpfs to within 64 KiB.
3. After the load, the file is removed and the node restarted. It must
   come back and catch up with the leader.

Pass: the driver saw no errors, every session finished exactly once
(`agentd-drive check`), the shard had a leader throughout, the node fenced
itself once its Raft hit the full disk (it stops claiming leadership and
exits), and it caught up after its disk had room again. Writes .shard/disk_full.json.
"""
import argparse, json, os, shutil, signal, subprocess, sys, time
import urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import fleet  # noqa: E402

DRIVE = os.environ.get("AGENTD_DRIVE_BIN", os.path.join(
    ROOT, os.environ.get("AGENTD_BIN_DIR", os.path.join("target", "release")), "agentd-drive"))

p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
p.add_argument("--rate", type=float, default=10)
p.add_argument("--secs", type=int, default=240)
p.add_argument("--fill-at", type=int, default=60)
p.add_argument("--node", choices=["leader", "follower"], default="follower")
p.add_argument("--tmpfs-mb", type=int, default=512)
p.add_argument("--max-catch-up-s", type=float, default=120.0)
p.add_argument("--seed-per-shard", type=int, default=0,
               help="sessions the shard holds before the disk fills (the verifier: goal density)")
a = p.parse_args()

# The proof needs to mount a tmpfs; without the rights it says so and
# records nothing (the verifier skips it), rather than failing the point.
probe = os.path.join("/tmp", f"disk-full-probe-{os.getpid()}")
os.makedirs(probe, exist_ok=True)
if subprocess.run(["mount", "-t", "tmpfs", "-o", "size=1m", "tmpfs", probe], capture_output=True).returncode != 0:
    print(json.dumps({"skipped": "needs root and mount rights (a privileged container)"}))
    sys.exit(77)
subprocess.run(["umount", probe])


def inspect(node):
    try:
        with urllib.request.urlopen(f"http://{fleet.addr(0, node)}/shard/inspect", timeout=2) as r:
            return json.load(r)
    except Exception:
        return None


subprocess.run([sys.executable, os.path.join(ROOT, "scripts/agentd/fleet.py"), "down"])
time.sleep(0.5)
for d in ("data/c0n1", "data/c0n2", "data/c0n3"):
    subprocess.run(["umount", os.path.join(fleet.STATE, d)], stderr=subprocess.DEVNULL)
shutil.rmtree(fleet.STATE, ignore_errors=True)
os.makedirs(fleet.STATE)
ledger = os.path.join(fleet.STATE, "ledger.txt")
os.environ.update({"AGENTD_LEDGER": ledger, "AGENTD_WORK_MS": "50"})
os.environ.setdefault("AGENTD_MEMORY_BUDGET_MB", "512")
fleet.up(1, 3, 7100, 2)
fleet.wait_ready()
seeded = fleet.seed(a.seed_per_shard)
leader = fleet.leader(0)
target = leader if a.node == "leader" else next(n for n in (1, 2, 3) if n != leader)

# 1. The target node's data onto a small tmpfs.
data = os.path.join(fleet.STATE, "data", f"c0n{target}")
fleet.kill(0, target, signal.SIGTERM)
time.sleep(2)
os.rename(data, data + ".disk")
os.makedirs(data)
held_mb = int(subprocess.run(["du", "-sm", data + ".disk"], capture_output=True, text=True).stdout.split()[0])
tmpfs_mb = max(a.tmpfs_mb, held_mb + 128)  # room for what it holds, then the load
subprocess.run(["mount", "-t", "tmpfs", "-o", f"size={tmpfs_mb}m", "tmpfs", data], check=True)
subprocess.run(["cp", "-a", data + ".disk/.", data], check=True)
fleet.start(0, target)
fleet.wait_ready()
print(f"node {target} ({a.node}) runs on a {tmpfs_mb} MB tmpfs", flush=True)

# 2. Load; fill the disk partway through.
# Clients reach the shard through the other members' gateways: the target's
# own gateway goes down with it once it fences itself, as a pod behind a
# Service stops getting traffic (prove_rejoin_under_load.py does the same).
gws = ",".join(f"http://{fleet.addr(0, n)}" for n in (1, 2, 3) if n != target)
sf = os.path.join(fleet.STATE, "sessions.txt")
drive = subprocess.Popen([DRIVE, "load", "--gateways", gws, "--agent", "worker", "--sessions", "0",
                          "--rate", str(a.rate), "--duration", str(a.secs), "--turns", "3", "--think", "5",
                          "--sessions-file", sf],
                         stdout=subprocess.PIPE, stderr=open(os.path.join(fleet.STATE, "drive.log"), "w"), text=True)
t0 = time.time()
filled_at, no_leader_s, timeline = None, 0, []
junk = os.path.join(data, "junk.fill")
while drive.poll() is None:
    t = round(time.time() - t0)
    if filled_at is None and t >= a.fill_at:
        st = os.statvfs(data)
        free = st.f_bavail * st.f_frsize
        with open(junk, "wb") as f:
            left = max(0, free - 64 * 1024)
            chunk = b"\0" * (1 << 20)
            while left > 0:
                n = min(left, len(chunk))
                try:
                    f.write(chunk[:n])
                except OSError:
                    break
                left -= n
        filled_at = t
        print(f"t={t}: node {target}'s disk is full", flush=True)
    lead = fleet.leader(0)
    if lead is None:
        no_leader_s += 2
    row = {"t": t, "leader": lead}
    for n in (1, 2, 3):
        r = inspect(n)
        row[n] = None if r is None else r.get("applied")
    timeline.append(row)
    time.sleep(2)
report = json.loads(drive.stdout.read() or "{}")
during = {"target_answers": inspect(target) is not None}

# 3. Room again; the node restarts and must catch up.
os.remove(junk)
fleet.kill(0, target, signal.SIGKILL)
time.sleep(1)
fleet.start(0, target)
back, caught_up_s = time.time(), None
while time.time() - back < a.max_catch_up_s:
    lead = fleet.leader(0)
    lr, tr = (inspect(lead) if lead else None), inspect(target)
    if lr and tr and tr.get("applied") is not None and lr.get("applied") - tr.get("applied") <= 64:
        caught_up_s = round(time.time() - back, 1)
        break
    time.sleep(1)
chk = subprocess.run([DRIVE, "check", "--gateways", gws, "--sessions-file", sf, "--turns", "3",
                      "--ledger", ledger], capture_output=True, text=True)
check = json.loads(chk.stdout) if chk.stdout.strip() else {"ok": False, "err": chk.stderr[-2000:]}
log = open(os.path.join(fleet.STATE, "logs", f"c0n{target}.log"), errors="replace").read()
res = {
    "node": target, "role": a.node, "tmpfs_mb": tmpfs_mb, "filled_at": filled_at,
    "sessions_per_shard": seeded,
    "sessions": report.get("sessions"), "turns_completed": report.get("turns_completed"),
    "errors": report.get("errors"), "first_errors": (report.get("first_errors") or [])[:5],
    "no_leader_s": no_leader_s, "during": during,
    "target_saw_enospc": "No space left on device" in log or "os error 28" in log,
    "target_fenced": "fencing this node" in log,
    "caught_up_s": caught_up_s,
    "check": {k: check.get(k) for k in ("ok", "failed_sessions", "failures", "err")},
    "timeline": timeline,
}
res["ok"] = (res["errors"] == 0 and bool(check.get("ok")) and caught_up_s is not None
             and no_leader_s <= 30 and (res["target_fenced"] or not res["target_saw_enospc"]))
json.dump(res, open(os.path.join(fleet.STATE, "disk_full.json"), "w"), indent=1)
subprocess.run([sys.executable, os.path.join(ROOT, "scripts/agentd/fleet.py"), "down"])
subprocess.run(["umount", data], stderr=subprocess.DEVNULL)
print(json.dumps({k: v for k, v in res.items() if k != "timeline"}, indent=1))
sys.exit(0 if res["ok"] else 1)
