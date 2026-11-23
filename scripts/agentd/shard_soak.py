#!/usr/bin/env python3
"""One shard of the 75k gate, scaled down, on one Linux machine.

  shard_soak.py                      # 1/8 of an EKS shard, about 35 minutes
  shard_soak.py --scale 0.05         # smaller and quicker
  shard_soak.py --out /tmp/soak      # where samples.jsonl and summary.json go

Decision: the 75k run on EKS is the check of last resort, not the way to find
a memory regression (it costs an hour and about $15, and the user asked for a
local emulation, 2026-11-23). What broke the gate in run 45179434646 was not
75 000 agents as such: it was one shard holding minutes of Raft log under
sustained load, then losing a node. That is one shard's shape, and it scales:

- sessions, the memory budget, the memory limit, the snapshot interval (in
  entries) and the pressure turn floor are the gate's per-shard values times
  `--scale`;
- arrival ramp, turns per session, think time and stream share stay the
  gate's, so the per-session rhythm and the log's growth over time match;
- at the end of the ramp (the gate's chaos moment) one node and half the
  executors are killed and come back after `--down-for` seconds, as a pod
  restart does;
- a node whose resident set passes its scaled limit is killed and restarted,
  as the kubelet's out-of-memory kill does, and counted. The run fails on
  any such kill and on any error the driver reports.

The memory figures come from /proc, which is what agentd's own pressure loop
reads, so this runs on Linux only (in a container on a Mac). Build first:

  cargo build --release -p everruns-agentd-testkit --bins
  AGENTD_BIN_DIR=target/release scripts/agentd/shard_soak.py
"""
import argparse, json, os, signal, subprocess, sys, threading, time, urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
BIN_DIR = os.path.join(ROOT, os.environ.get("AGENTD_BIN_DIR", os.path.join("target", "release")))
os.environ.setdefault("AGENTD_NODE_BIN", os.path.join(BIN_DIR, "agentd-testnode"))
import fleet  # noqa: E402

DRIVE = os.environ.get("AGENTD_DRIVE_BIN", os.path.join(BIN_DIR, "agentd-drive"))

# One shard of the EKS gate (saas dataplane-load.yml defaults with the
# 2026-11-23 gate's inputs): 75 000 sessions over 3 shards, 9 GiB budget in a
# 14 GiB container, 8 executors for 3 shards.
GATE = {
    "sessions_per_shard": 25_000,
    "budget_mb": 9216,
    "limit_mb": 14336,
    "snapshot_every": 20_000,
    "pressure_turns": 1024,
    "ramp": 600,
    "turns": 10,
    "think": 60,
    "stream": 0.2,
}
NODES = (1, 2, 3)
EXECUTORS = 2


def rss_mb(pid):
    try:
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                if line.startswith("VmRSS:"):
                    return int(line.split()[1]) / 1024
    except (FileNotFoundError, ProcessLookupError):
        pass
    return 0.0


def node_pid(n):
    try:
        with open(fleet.pidfile(0, n)) as f:
            return int(f.read())
    except (FileNotFoundError, ValueError):
        return None


def stats(n):
    try:
        return fleet.stats(0, n, timeout=1.0)
    except Exception:
        return None


def scaled(a):
    s = a.scale
    return {
        "sessions": max(30, int(GATE["sessions_per_shard"] * s)),
        "budget_mb": int(GATE["budget_mb"] * s),
        "limit_mb": int(GATE["limit_mb"] * s),
        "snapshot_every": max(100, int(GATE["snapshot_every"] * s)),
        "pressure_turns": max(16, int(GATE["pressure_turns"] * s)),
    }


def start_fleet(cfg):
    os.environ["AGENTD_MEMORY_BUDGET_MB"] = str(cfg["budget_mb"])
    os.environ["AGENTD_MEMORY_LIMIT_MB"] = str(cfg["limit_mb"])
    os.environ["AGENTD_PRESSURE_TURNS"] = str(cfg["pressure_turns"])
    os.environ["AGENTD_NODE_ARGS"] = (
        os.environ.get("AGENTD_NODE_ARGS", "") + f" --snapshot-every {cfg['snapshot_every']}").strip()
    subprocess.run([sys.executable, os.path.join(ROOT, "scripts/agentd/fleet.py"), "down"])
    time.sleep(0.5)
    subprocess.run(["rm", "-rf", fleet.STATE])
    fleet.up(1, len(NODES), 7100, executors=EXECUTORS)


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--scale", type=float, default=0.125, help="share of one EKS gate shard")
    p.add_argument("--ramp", type=int, default=GATE["ramp"])
    p.add_argument("--turns", type=int, default=GATE["turns"])
    p.add_argument("--think", type=float, default=GATE["think"])
    p.add_argument("--stream", type=float, default=GATE["stream"])
    p.add_argument("--agent", default="mix")
    p.add_argument("--kill-at", type=int, default=None, help="seconds in (default: end of ramp)")
    p.add_argument("--down-for", type=int, default=15, help="seconds a killed node stays down")
    p.add_argument("--no-chaos", action="store_true")
    p.add_argument("--every", type=float, default=0.5, help="sample interval, seconds")
    p.add_argument("--out", default=os.path.join(ROOT, "target", "shard-soak"))
    p.add_argument("--keep", action="store_true", help="leave the fleet running")
    a = p.parse_args()
    if not os.path.exists("/proc/self/status"):
        sys.exit("needs Linux (/proc): run it in a container on other systems")
    cfg = scaled(a)
    kill_at = a.ramp if a.kill_at is None else a.kill_at
    os.makedirs(a.out, exist_ok=True)
    samples = open(os.path.join(a.out, "samples.jsonl"), "w")
    print(json.dumps({"scale": a.scale, **cfg}), flush=True)

    start_fleet(cfg)
    started = time.time()
    drive = subprocess.Popen(
        [DRIVE, "load", "--gateways", ",".join(f"http://{fleet.addr(0, n)}" for n in NODES),
         "--agent", a.agent, "--sessions", str(cfg["sessions"]), "--turns", str(a.turns),
         "--concurrency", str(cfg["sessions"]), "--think", str(a.think), "--stream", str(a.stream),
         "--persistent", "--rate", f"{cfg['sessions'] / max(a.ramp, 1):.3f}",
         "--duration", str(a.ramp)],
        stdout=subprocess.PIPE, stderr=open(os.path.join(a.out, "drive.log"), "w"), text=True)
    report = {}
    reader = threading.Thread(target=lambda: report.update(json.loads(drive.stdout.read() or "{}")))
    reader.start()

    oom = []        # (t, node, rss) for every emulated out-of-memory kill
    chaos = []
    down_until = {}  # node -> time it comes back
    peak = {n: 0.0 for n in NODES}
    peak_log = {n: 0 for n in NODES}
    try:
        while drive.poll() is None:
            now = time.time()
            t = round(now - started, 1)
            if not a.no_chaos and not chaos and t >= kill_at:
                victim = NODES[0]
                fleet.kill(0, victim)
                fleet.kill_executor(1)
                down_until[victim] = now + a.down_for
                down_until["e1"] = now + a.down_for
                chaos.append({"t": t, "killed": [f"node {victim}", "executor 1"]})
                print(f"t={t}: killed node {victim} and executor 1", flush=True)
            for key, until in list(down_until.items()):
                if now >= until:
                    if key == "e1":
                        fleet.start_executor(1)
                    else:
                        fleet.start(0, key)
                    del down_until[key]
            row = {"t": t, "nodes": {}}
            for n in NODES:
                pid = node_pid(n)
                rss = rss_mb(pid) if pid else 0.0
                st = stats(n) or {}
                row["nodes"][n] = {"rss_mb": round(rss), "log_mb": st.get("wal_held_mb"),
                                   "resident_mb": st.get("resident_mb"),
                                   "sessions": st.get("sessions"),
                                   "active_turns": st.get("active_turns"),
                                   "queued_inputs": st.get("queued_inputs")}
                peak[n] = max(peak[n], rss)
                peak_log[n] = max(peak_log[n], st.get("wal_held_mb") or 0)
                if pid and rss > cfg["limit_mb"] and n not in down_until:
                    # What the kubelet does to a container past its limit.
                    fleet.kill(0, n)
                    down_until[n] = now + 5
                    oom.append({"t": t, "node": n, "rss_mb": round(rss)})
                    print(f"t={t}: node {n} out of memory at {round(rss)} MB "
                          f"(limit {cfg['limit_mb']})", flush=True)
            samples.write(json.dumps(row) + "\n")
            samples.flush()
            time.sleep(a.every)
    finally:
        reader.join(timeout=30)
        if not a.keep:
            subprocess.run([sys.executable, os.path.join(ROOT, "scripts/agentd/fleet.py"), "down"])
    summary = {
        "scale": a.scale,
        "config": cfg,
        "secs": round(time.time() - started),
        "turns": report.get("turns_completed"),
        "errors": report.get("errors"),
        "chaos": chaos,
        "oom_kills": oom,
        "peak_rss_mb": {n: round(v) for n, v in peak.items()},
        "peak_rss_share_of_limit": round(max(peak.values()) / cfg["limit_mb"], 3),
        "peak_log_mb": peak_log,
    }
    with open(os.path.join(a.out, "summary.json"), "w") as f:
        json.dump(summary, f, indent=2)
    print(json.dumps(summary, indent=2))
    if oom or report.get("errors") or drive.returncode:
        sys.exit(f"failed: {len(oom)} out-of-memory kills, {report.get('errors')} errors, "
                 f"driver exit {drive.returncode}")


if __name__ == "__main__":
    main()
