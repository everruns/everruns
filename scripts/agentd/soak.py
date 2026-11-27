#!/usr/bin/env python3
"""A long, production-shaped run on a local fleet: steady arrivals, think
time between messages, real tool timings, optional node kills.

  soak.py                                            # 30 min, 2 shards, 10 new sessions/s
  soak.py --minutes 60 --rate 20 --think 20 --turns 5 --kill-every 300
  soak.py --minutes 5 --rate 5                       # a quick look

Sessions arrive at `--rate` per second (Poisson) for `--minutes`, each runs
`--turns` turns of the llmsim scenario mix with an exponential pause of
`--think` seconds (mean) between them, so the fleet holds a steady
population of active sessions, most of them idle between messages, like
users. Every 30 s it samples each shard (running turns, sessions, unloaded
sessions) and each node's RSS. With `--kill-every`, a random node is killed
with SIGKILL and restarted at that interval.

At the end every session is checked: each message completed exactly once,
sequences are dense, and no tool ran more times than it has durable
`tool.started` events. The timeline goes to .shard/soak.json.
"""
import argparse, json, os, random, subprocess, sys, threading, time, urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "scripts", "shard"))
import fleet  # noqa: E402

DRIVE = os.path.join(ROOT, "target", "release", "agentd-drive")


def get(url, timeout=3):
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return r.read().decode()


def rss_mb(shard, node):
    try:
        for line in get(f"http://{fleet.addr(shard, node)}/metrics").splitlines():
            if line.startswith("agentd_shard_rss_bytes"):
                return round(float(line.rsplit(" ", 1)[1]) / 2**20)
    except Exception:
        return None


def shard_sample(shard):
    for node in fleet.nodes_of(shard):
        try:
            load = json.loads(get(f"http://{fleet.addr(shard, node)}/shard/load"))
            stats = json.loads(get(f"http://{fleet.addr(shard, node)}/shard/stats"))
            return {"running": load["active"], "queued": load["queued"],
                    "sessions": load["sessions"], "unloaded": stats.get("unloaded_sessions")}
        except Exception:
            pass
    return None


def cgroup_limit_mb():
    """The tightest memory limit on this process's cgroup v2 path, in MiB.

    Decision (R9, 2026-11-26): `/sys/fs/cgroup` is the root of the
    hierarchy, not this container's cgroup, wherever the container shares
    the host's cgroup namespace (a privileged pod in kind): its
    `memory.max` there is the kind node's 32 GiB, and the soak gave six
    nodes 29.5 GB in a 27 GiB pod. The own cgroup is named in
    `/proc/self/cgroup`; every level of it up to the root can carry a
    limit, and the tightest one is what the kernel enforces.
    """
    try:
        own = next(l.split(":", 2)[2].strip() for l in open("/proc/self/cgroup") if l.startswith("0::"))
    except (OSError, StopIteration):
        return 0
    path, best = "/sys/fs/cgroup" + own.rstrip("/"), 0
    while path.startswith("/sys/fs/cgroup"):
        try:
            raw = open(os.path.join(path, "memory.max")).read().strip()
            if raw.isdigit():
                mb = int(raw) // 2**20
                best = mb if not best else min(best, mb)
        except OSError:
            pass
        if path == "/sys/fs/cgroup":
            break
        path = os.path.dirname(path)
    return best


def machine_mb():
    """This container's memory limit (cgroup v2, then v1), else the machine's."""
    own = cgroup_limit_mb()
    if own:
        return own
    for path in ("/sys/fs/cgroup/memory/memory.limit_in_bytes",):
        try:
            raw = open(path).read().strip()
        except OSError:
            continue
        if raw.isdigit() and int(raw) < 1 << 60:
            return int(raw) // 2**20
    try:
        return os.sysconf("SC_PAGE_SIZE") * os.sysconf("SC_PHYS_PAGES") // 2**20
    except (ValueError, OSError):
        return 0


def node_limit_mb(nodes):
    """Each node's share of the machine, as its memory limit.

    Decision (R9, 2026-11-26): with no limit a node never reaches its
    pressure levels, so an overloaded fleet held every turn it admitted and
    grew until the kernel killed it (6 nodes, 3 to 6 GB each, 27 GiB pod,
    24 minutes in). A production node always runs with a limit
    (`AGENTD_MEMORY_LIMIT_MB`) and sheds new work from 92% of it; the soak
    gives each node its share, nine tenths of the machine split evenly,
    leaving the rest to the load driver and the kernel. The pass rule is
    unchanged: every session finishes all its turns, 0 errors.
    """
    return int(machine_mb() * 0.9 / max(nodes, 1))


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--shards", type=int, default=2)
    p.add_argument("--minutes", type=float, default=30)
    p.add_argument("--rate", type=float, default=10, help="new sessions per second")
    p.add_argument("--turns", type=int, default=5, help="turns per session")
    p.add_argument("--think", type=float, default=15, help="mean seconds between a session's turns")
    p.add_argument("--scenarios", default="mix")
    p.add_argument("--scale", default="1", help="AGENTD_TOOL_SCALE (1 = real tool durations)")
    p.add_argument("--memory-mb", default="128", help="memory budget per node")
    p.add_argument("--limit-mb", type=int, default=0,
                   help="memory limit per node (0: its share of this machine, see node_limit_mb)")
    p.add_argument("--kill-every", type=float, default=0, help="seconds between kills (0: none)")
    p.add_argument("--seed-per-shard", type=int, default=0, help="sessions each shard holds before the fault (the verifier: goal density)")
    a = p.parse_args()

    subprocess.run([sys.executable, os.path.join(ROOT, "scripts/agentd/fleet.py"), "down"])
    time.sleep(0.5)
    subprocess.run(["rm", "-rf", fleet.STATE])
    os.makedirs(fleet.STATE, exist_ok=True)
    ledger = os.path.join(fleet.STATE, "ledger.txt")
    limit_mb = a.limit_mb or int(os.environ.get("AGENTD_MEMORY_LIMIT_MB") or 0) or node_limit_mb(3 * a.shards)
    os.environ.update({"AGENTD_LEDGER": ledger, "AGENTD_TOOL_SCALE": a.scale,
                       "AGENTD_MEMORY_BUDGET_MB": a.memory_mb,
                       "AGENTD_MEMORY_LIMIT_MB": str(limit_mb)})
    fleet.up(a.shards, 3, 7100)
    seeded = fleet.seed(a.seed_per_shard)
    time.sleep(1)

    gateways = ",".join(f"http://{fleet.addr(c, n)}" for c in range(a.shards) for n in (1, 2, 3))
    sessions_file = os.path.join(fleet.STATE, "sessions.txt")
    out = os.path.join(fleet.STATE, "soak-load.json")
    print(f"soak: {a.minutes:g} min, {a.rate:g} new sessions/s, {a.turns} turns each, "
          f"think {a.think:g} s, tool scale {a.scale}, {a.shards} shards, {limit_mb} MB a node", flush=True)
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
                   "shards": {c: shard_sample(c) for c in range(a.shards)},
                   "rss_mb": {f"c{c}n{n}": rss_mb(c, n) for c in range(a.shards) for n in (1, 2, 3)}}
            timeline.append(row)
            shards = "  ".join(f"shard {c}: {s['running']:>4} running {s['sessions']:>6} sessions "
                              f"({s['unloaded'] or 0} unloaded)"
                              for c, s in row["shards"].items() if s)
            rss = [v for v in row["rss_mb"].values() if v]
            print(f"{row['t']:>6}s  {shards}  RSS max {max(rss) if rss else '?'} MB", flush=True)

    def chaos():
        while not stop.wait(a.kill_every):
            shard, node = random.randrange(a.shards), random.choice((1, 2, 3))
            was_leader = fleet.leader(shard) == node
            fleet.kill(shard, node)
            kills.append({"t": round(time.time() - started), "shard": shard, "node": node,
                          "leader": was_leader})
            print(f"kill -9 shard {shard} node {node} (leader={was_leader})", flush=True)
            time.sleep(random.uniform(2, 5))
            fleet.start(shard, node)

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
    report.pop("shards", None)
    peak = lambda key: max((sum((s or {}).get(key) or 0 for s in r["shards"].values()) for r in timeline), default=0)  # noqa: E731
    rss = [v for r in timeline for v in r["rss_mb"].values() if v]
    summary = {
        "sessions_per_shard": seeded,
        "minutes": a.minutes, "rate": a.rate, "turns_per_session": a.turns, "think_secs": a.think,
        "tool_scale": a.scale, "shards": a.shards,
        "sessions": report["sessions"], "turns": report["turns_completed"], "errors": report["errors"],
        "turns_per_sec": round(report["turns_per_sec"], 1),
        "turn": report["turn"], "admit": report["admit"], "scenarios": report["scenarios"],
        "peak_running_turns": peak("running"), "peak_sessions": peak("sessions"),
        "peak_unloaded": peak("unloaded"),
        "rss_mb": {"max": max(rss, default=None),
                   "last": {k: v for k, v in timeline[-1]["rss_mb"].items()} if timeline else None},
        "kills": len(kills), "leader_kills": sum(k["leader"] for k in kills),
        "limit_mb": limit_mb,
        # Refusals for load (429, retried with backoff until they finish):
        # not errors, and their share of the requests sent.
        "shed": report.get("shed") or 0,
        "shed_share": round((report.get("shed") or 0) / max(
            (report.get("sessions") or 0) + (report.get("turns_completed") or 0) + (report.get("shed") or 0), 1), 5),
        "check": {k: result.get(k) for k in ("ok", "failed_sessions", "totals", "failures")},
    }
    with open(os.path.join(fleet.STATE, "soak.json"), "w") as f:
        json.dump({"summary": summary, "timeline": timeline, "kills": kills}, f, indent=1)
    print(json.dumps(summary, indent=2))
    sys.exit(0 if result.get("ok") and report["errors"] == 0 else 1)


if __name__ == "__main__":
    main()
