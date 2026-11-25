#!/usr/bin/env python3
"""Nodes at 98% of their memory limit restart themselves, one replica at a
time, leadership moves, and no turn is lost (75k verifier point U13).

  prove_memory_restart.py [--sessions 1000] [--limit-mb 4096] [--seed-per-shard 0]

1. One shard (3 nodes, 2 executors), each node with `--limit-mb` as its
   memory limit, runs `--sessions` `worker` sessions through `--turns`
   turns, with the work tool's ledger.
2. Under that load the leader and one follower are driven toward their
   limit together: the test node binary holds as much ballast memory as the
   proof asks (`AGENTD_BALLAST_DIR`, testnode.rs), raised a step at a time
   until the node reads 98.5%. The shard passes through Full, Shedding and
   Emergency on the way, and each node's controlled restart is due at 98%
   (`pressure.rs`, level 4).
3. A node that exits with 75 is started again from its disk, as
   Kubernetes does. A node whose resident set passes its limit is killed
   and counted, as the kernel would (`shard_soak.py` does the same).
4. When the load is done, `agentd-drive check` with the ledger: each
   message completed once, sequences dense.

Pass: both driven nodes restarted themselves, never two at once (the
restart is exclusive across replicas, codex review 2026-11-24); the leader
was one of them and another node led after it; no out-of-memory kill, no
crash, no load error, and the check holds. Writes .shard/memory_restart.json.

Linux only (the proof reads /proc). Build first:
  cargo build --release -p everruns-agentd-testkit --bins
"""
import argparse, json, os, shutil, subprocess, sys, threading, time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import fleet  # noqa: E402

DRIVE = os.environ.get("AGENTD_DRIVE_BIN", os.path.join(
    ROOT, os.environ.get("AGENTD_BIN_DIR", os.path.join("target", "release")), "agentd-drive"))
RESTART_EXIT = 75  # memwatch::RESTART_EXIT
RESTART_LINE = "restarting before the kernel kills this node"
NODES = (1, 2, 3)

p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
p.add_argument("--sessions", type=int, default=1000)
p.add_argument("--turns", type=int, default=6)
p.add_argument("--think", type=float, default=5.0)
p.add_argument("--limit-mb", type=int, default=4096, help="each node's memory limit")
p.add_argument("--warm-s", type=int, default=20, help="load before the pressure starts")
p.add_argument("--step-share", type=float, default=0.01, help="ballast added per step, share of the limit")
p.add_argument("--max-s", type=int, default=900, help="give up waiting for both restarts after this")
p.add_argument("--seed-per-shard", type=int, default=0, help="sessions the shard holds before the fault (the verifier: goal density)")
a = p.parse_args()
if not os.path.exists("/proc/self/status"):
    sys.exit("needs Linux (/proc): run it in a container on other systems")


def rss_mb(pid):
    try:
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                if line.startswith("VmRSS:"):
                    return int(line.split()[1]) / 1024
    except (FileNotFoundError, ProcessLookupError):
        pass
    return 0.0


def pid_of(n):
    try:
        with open(fleet.pidfile(0, n)) as f:
            return int(f.read())
    except (FileNotFoundError, ValueError):
        return None


def exited(pid):
    """None while the process runs, else its exit code (-1: reaped elsewhere,
    code unknown)."""
    try:
        done, status = os.waitpid(pid, os.WNOHANG)
    except ChildProcessError:
        try:
            os.kill(pid, 0)
            return None
        except ProcessLookupError:
            return -1
    return os.waitstatus_to_exitcode(status) if done else None


def restart_lines(n):
    try:
        with open(os.path.join(fleet.STATE, "logs", f"c0n{n}.log"), errors="replace") as f:
            return f.read().count(RESTART_LINE)
    except FileNotFoundError:
        return 0


def stats(n):
    try:
        return fleet.stats(0, n, timeout=1.0)
    except Exception:
        return None


subprocess.run([sys.executable, os.path.join(ROOT, "scripts/agentd/fleet.py"), "down"])
time.sleep(0.5)
shutil.rmtree(fleet.STATE, ignore_errors=True)
os.makedirs(fleet.STATE)
ledger = os.path.join(fleet.STATE, "ledger.txt")
ballast_dir = os.path.join(fleet.STATE, "ballast")
os.makedirs(ballast_dir)
os.environ.update({
    "AGENTD_LEDGER": ledger, "AGENTD_WORK_MS": "50",
    "AGENTD_MEMORY_LIMIT_MB": str(a.limit_mb),
    "AGENTD_BALLAST_DIR": ballast_dir,
})
os.environ.setdefault("AGENTD_MEMORY_BUDGET_MB", str(a.limit_mb // 2))
fleet.up(1, len(NODES), 7100, 2)
fleet.wait_ready()
seeded = fleet.seed(a.seed_per_shard)
gws = [f"http://{fleet.addr(0, n)}" for n in NODES]
sf = os.path.join(fleet.STATE, "sessions.txt")

drive = subprocess.Popen(
    [DRIVE, "load", "--gateways", ",".join(gws), "--agent", "worker",
     "--sessions", str(a.sessions), "--turns", str(a.turns), "--think", str(a.think),
     "--concurrency", str(a.sessions), "--sessions-file", sf, "--persistent"],
    stdout=subprocess.PIPE, stderr=open(os.path.join(fleet.STATE, "drive.log"), "w"), text=True)
report = {}
reader = threading.Thread(target=lambda: report.update(json.loads(drive.stdout.read() or "{}")))
reader.start()
t0 = time.time()
time.sleep(a.warm_s)

first_leader = fleet.leader(0)
driven = [first_leader, next(n for n in NODES if n != first_leader)]
print(json.dumps({"t": round(time.time() - t0), "leader": first_leader, "driving": driven}), flush=True)
ballast = {n: 0 for n in driven}
pressing = set(driven)
seen_lines = {n: restart_lines(n) for n in NODES}
restarts, crashes, ooms = [], [], []
down = {}        # node -> t it went down; cleared once it answers and caught up
intervals = []   # (node, down t, back t)
overlap = 0
leader = first_leader
peak_share = {n: 0.0 for n in NODES}
timeline = []
step = max(1, int(a.limit_mb * a.step_share))
deadline = time.time() + a.max_s
while time.time() < deadline:
    t = round(time.time() - t0, 1)
    lead = fleet.leader(0)
    if lead is not None:
        leader = lead
    row = {"t": t, "leader": lead, "nodes": {}}
    for n in NODES:
        if n in down:
            st = stats(n)
            lead_st = stats(leader) if leader and leader != n else None
            if st and st.get("leader") is not None and lead_st and \
                    st.get("applied", 0) + 100 >= lead_st.get("applied", 0):
                intervals.append((n, down.pop(n), t))
                print(f"t={t}: node {n} back and caught up", flush=True)
            continue
        pid = pid_of(n)
        code = exited(pid) if pid else -1
        if code is not None:
            lines = restart_lines(n)
            controlled = code == RESTART_EXIT or (code == -1 and lines > seen_lines[n])
            seen_lines[n] = lines
            if any(m for m in down if m != n):
                overlap += 1
            (restarts if controlled else crashes).append(
                {"t": t, "node": n, "was_leader": n == leader, "code": code})
            print(f"t={t}: node {n} exited ({code}{', controlled' if controlled else ''}); starting it again", flush=True)
            down[n] = t
            pressing.discard(n)  # its new process holds no ballast
            time.sleep(1)
            fleet.start(0, n)
            continue
        rss = rss_mb(pid)
        share = rss / a.limit_mb
        peak_share[n] = max(peak_share[n], share)
        row["nodes"][n] = {"rss_mb": round(rss), "ballast_mb": ballast.get(n, 0) if n in pressing else 0}
        if rss > a.limit_mb:
            # What the kubelet does to a container past its limit.
            fleet.kill(0, n)
            ooms.append({"t": t, "node": n, "rss_mb": round(rss)})
            print(f"t={t}: node {n} out of memory at {round(rss)} MB", flush=True)
            down[n] = t
            pressing.discard(n)
            fleet.start(0, n)
            continue
        if n in pressing and share < 0.985:
            ballast[n] += step
            with open(os.path.join(ballast_dir, str(pid)), "w") as f:
                f.write(str(ballast[n]))
    timeline.append(row)
    if not pressing and not down and len(restarts) + len(crashes) + len(ooms) >= len(driven):
        break
    time.sleep(0.5)

drive.wait()
reader.join(timeout=30)
chk = subprocess.run([DRIVE, "check", "--gateways", ",".join(gws), "--sessions-file", sf,
                      "--turns", str(a.turns), "--ledger", ledger], capture_output=True, text=True)
check = json.loads(chk.stdout) if chk.stdout.strip() else {"ok": False, "err": chk.stderr[-2000:]}
last_leader = fleet.leader(0)

restarted = {r["node"] for r in restarts}
leader_restarted = any(r["was_leader"] for r in restarts)
# Another node led once the leader had gone (its restart stops Raft, so the
# followers elect at once rather than after a lost heartbeat).
handed = [r for r in restarts if r["was_leader"]]
handed_over = bool(handed) and any(
    row["leader"] not in (None, handed[0]["node"]) for row in timeline if row["t"] >= handed[0]["t"])
res = {
    "sessions_per_shard": seeded,
    "sessions": a.sessions, "limit_mb": a.limit_mb,
    "first_leader": first_leader, "last_leader": last_leader, "driven": driven,
    "restarts": restarts, "crashes": crashes, "oom_kills": ooms,
    "overlapping_restarts": overlap, "leader_handed_over": handed_over,
    "down_s": [{"node": n, "s": round(b - d, 1)} for n, d, b in intervals],
    "peak_share_of_limit": {n: round(v, 3) for n, v in peak_share.items()},
    "load": {k: report.get(k) for k in ("sessions", "turns_completed", "errors", "stalled")},
    "check": {k: check.get(k) for k in ("ok", "failed_sessions", "totals", "failures", "err")},
    "wall_s": round(time.time() - t0),
}
res["ok"] = (set(driven) <= restarted and overlap == 0 and leader_restarted and handed_over
             and not crashes and not ooms and bool(check.get("ok"))
             and (report.get("errors") or 0) == 0)
res["errors"] = report.get("errors") or 0
json.dump({**res, "timeline": timeline}, open(os.path.join(fleet.STATE, "memory_restart.json"), "w"), indent=1)
subprocess.run([sys.executable, os.path.join(ROOT, "scripts/agentd/fleet.py"), "down"])
print(json.dumps(res, indent=1))
sys.exit(0 if res["ok"] else 1)
