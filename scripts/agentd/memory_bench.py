#!/usr/bin/env python3
"""What a shard's memory and uploads cost per session, against a recorded baseline.

  memory_bench.py                 # run, compare with the baseline, exit 1 when worse
  memory_bench.py --record        # run and write the baseline

Decision: the 75k run on EKS is the gate before a release, not the check for
every storage or memory change. It takes an hour and a few dollars, and the
Phase 2 snapshot regression (all three replicas of a shard out of memory at
the same entry, 2026-11-22) needed it to show up. This bench shows the same
shapes on one machine: one shard of three nodes and two executors, 2 000
sessions of the step 11.7 feature mix on simulated models, 4 turns each
(about 15 minutes on a 4-core box with a debug build), then

- node memory per thousand sessions (resident set after the load, over the
  idle fleet), gated on the worst node;
- what one snapshot build adds on top of the node at its peak, on a follower
  and on the leader (which also uploads it), measured with the kernel's peak
  resident set (`/proc/<pid>/clear_refs` 5 resets it);
- object store writes per turn, by kind, over every process.

Each gated figure fails at more than 10% over the baseline. Two runs of one
build moved by 5% (worst node), 2% (snapshots) and under 1% (writes); the
pre-fix snapshot build (8e4d1aef) measured a follower snapshot 42% over. The baseline
holds the build profile it was recorded with: a debug and a release build
lay out memory differently, so they are not compared. Build first:

  cargo build -p everruns-agentd-testkit --bins            # debug
  AGENTD_BIN_DIR=target/release memory_bench.py            # or a release build
"""
import argparse, json, os, re, subprocess, sys, time, urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
BIN_DIR = os.path.join(ROOT, os.environ.get("AGENTD_BIN_DIR", os.path.join("target", "debug")))
os.environ.setdefault("AGENTD_NODE_BIN", os.path.join(BIN_DIR, "agentd-testnode"))
import fleet  # noqa: E402

DRIVE = os.environ.get("AGENTD_DRIVE_BIN", os.path.join(BIN_DIR, "agentd-drive"))
BASELINE = os.path.join(ROOT, "scripts", "agentd", "memory_bench_baseline.json")
NODES = (1, 2, 3)
EXECUTORS = 2
TOLERANCE = 0.10
# Figures this small move by whole units between runs; below the floor a
# change is noise, not a regression.
FLOORS = {"rss_mb_per_1k_sessions": 5.0, "snapshot_mb": 8.0, "puts_per_turn": 0.05}
PUTS = re.compile(r'^agentd_store_puts_total\{[^}]*kind="([a-z]+)"[^}]*\} ([0-9.e+]+)$')


def proc_kb(pid, field):
    with open(f"/proc/{pid}/status") as f:
        for line in f:
            if line.startswith(field + ":"):
                return int(line.split()[1])
    return 0


def rss_mb(pid):
    return proc_kb(pid, "VmRSS") / 1024


def node_pid(n):
    with open(fleet.pidfile(0, n)) as f:
        return int(f.read())


def puts(url):
    out = {}
    text = urllib.request.urlopen(url, timeout=10).read().decode()
    for line in text.splitlines():
        m = PUTS.match(line)
        if m:
            out[m.group(1)] = out.get(m.group(1), 0) + float(m.group(2))
    return out


def all_puts():
    total = {}
    urls = [f"http://{fleet.addr(0, n)}/metrics" for n in NODES]
    urls += [f"http://127.0.0.1:{7000 + e}/metrics" for e in range(1, EXECUTORS + 1)]
    for url in urls:
        for kind, n in puts(url).items():
            total[kind] = total.get(kind, 0) + n
    return total


def settle(seconds):
    """Wait for the fleet to go quiet: jemalloc purges freed pages after a second."""
    time.sleep(seconds)


def snapshot_peak(n):
    """What one snapshot build on node `n` adds over its resident set."""
    pid = node_pid(n)
    path = os.path.join(fleet.STATE, "data", f"c0n{n}", "snapshots", "current.snap")
    before_file = os.path.getmtime(path) if os.path.exists(path) else 0
    settle(2)
    base = rss_mb(pid)
    with open(f"/proc/{pid}/clear_refs", "w") as f:
        f.write("5")
    req = urllib.request.Request(f"http://{fleet.addr(0, n)}/shard/snapshot", method="POST")
    urllib.request.urlopen(req, timeout=60).read()
    deadline = time.time() + 300
    while time.time() < deadline:
        if os.path.exists(path) and os.path.getmtime(path) > before_file:
            break
        time.sleep(0.2)
    else:
        sys.exit(f"node {n} wrote no snapshot in 5 minutes")
    # The leader reads the file back and uploads it after the rename.
    settle(3)
    return max(0.0, proc_kb(pid, "VmHWM") / 1024 - base)


def run(a):
    subprocess.run([sys.executable, os.path.join(ROOT, "scripts/agentd/fleet.py"), "down"])
    time.sleep(0.5)
    subprocess.run(["rm", "-rf", fleet.STATE])
    fleet.up(1, 3, 7100, executors=EXECUTORS)
    settle(3)
    idle = {n: rss_mb(node_pid(n)) for n in NODES}
    puts_before = all_puts()
    started = time.time()
    out = subprocess.run(
        [DRIVE, "load", "--gateways", ",".join(f"http://{fleet.addr(0, n)}" for n in NODES),
         "--agent", "mix", "--sessions", str(a.sessions), "--turns", str(a.turns),
         "--concurrency", str(a.concurrency), "--think", str(a.think), "--stream", "0.2"],
        capture_output=True, text=True)
    if out.returncode != 0:
        sys.exit(f"the load failed:\n{out.stderr[-4000:]}")
    report = json.loads(out.stdout)
    load_secs = time.time() - started
    settle(5)
    puts_after = all_puts()
    leader = fleet.leader(0)
    followers = [n for n in NODES if n != leader]
    loaded = {n: rss_mb(node_pid(n)) for n in NODES}
    stats = fleet.stats(0, leader, timeout=10)
    per_1k = {n: (loaded[n] - idle[n]) * 1000 / a.sessions for n in NODES}
    follower_snapshot = snapshot_peak(followers[0])
    leader_snapshot = snapshot_peak(leader)
    turns = report["turns_completed"]
    kinds = {k: (puts_after.get(k, 0) - puts_before.get(k, 0)) / max(turns, 1)
             for k in sorted(set(puts_after) | set(puts_before))}
    return {
        "profile": os.path.basename(os.path.normpath(BIN_DIR)),
        "sessions": a.sessions,
        "turns": turns,
        "errors": report["errors"],
        "load_secs": round(load_secs),
        "turn_p99_ms": report["turn"]["p99_ms"],
        "resident_mb": stats.get("resident_mb"),
        "resident_events": stats.get("resident_events"),
        "wal_held_mb": stats.get("wal_held_mb"),
        "rss_mb": {"idle": {n: round(idle[n]) for n in NODES}, "loaded": {n: round(loaded[n]) for n in NODES},
                   "leader": leader},
        # Which node holds more moves between runs by up to 15% (allocator
        # and leadership timing); the worst node moves by about 5%.
        "rss_mb_per_1k_sessions": {"leader": round(per_1k[leader], 1),
                                   "followers": [round(per_1k[n], 1) for n in followers]},
        "gated": {
            "node_rss_mb_per_1k_sessions": round(max(per_1k.values()), 1),
            "follower_snapshot_mb": round(follower_snapshot, 1),
            "leader_snapshot_mb": round(leader_snapshot, 1),
            "puts_per_turn": round(sum(kinds.values()), 3),
        },
        "puts_per_turn_by_kind": {k: round(v, 3) for k, v in kinds.items() if v},
    }


def floor(name):
    for suffix, value in FLOORS.items():
        if name.endswith(suffix):
            return value
    return 0.0


def compare(result, base):
    if base.get("profile") != result["profile"]:
        sys.exit(f"the baseline is a {base.get('profile')} build and this is {result['profile']}: "
                 "record one for this build (--record)")
    if (base["sessions"], base.get("turns_per_session")) != (result["sessions"], result.get("turns_per_session")):
        sys.exit("the baseline ran a different load: run with its --sessions and --turns, or --record")
    verdicts = {}
    for name, value in result["gated"].items():
        was = base["gated"].get(name)
        if was is None:
            continue
        limit = max(was * (1 + TOLERANCE), was + floor(name))
        verdicts[name] = {"value": value, "baseline": was, "limit": round(limit, 3), "ok": value <= limit}
    return verdicts


def main():
    p = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    p.add_argument("--sessions", type=int, default=2000)
    p.add_argument("--turns", type=int, default=4)
    p.add_argument("--concurrency", type=int, default=400)
    p.add_argument("--think", type=float, default=0.5, help="mean pause between a session's turns, seconds")
    p.add_argument("--record", action="store_true", help="write the result as the baseline")
    p.add_argument("--keep", action="store_true", help="leave the fleet running")
    a = p.parse_args()
    try:
        result = run(a)
    finally:
        if not a.keep:
            subprocess.run([sys.executable, os.path.join(ROOT, "scripts/agentd/fleet.py"), "down"])
    result["turns_per_session"] = a.turns
    if result["errors"]:
        print(json.dumps(result, indent=2))
        sys.exit(f"the load had {result['errors']} errors")
    if a.record:
        result["recorded"] = time.strftime("%Y-%m-%d")
        with open(BASELINE, "w") as f:
            json.dump(result, f, indent=2)
            f.write("\n")
        print(json.dumps(result, indent=2))
        return
    with open(BASELINE) as f:
        base = json.load(f)
    result["verdicts"] = compare(result, base)
    print(json.dumps(result, indent=2))
    worse = [k for k, v in result["verdicts"].items() if not v["ok"]]
    if worse:
        sys.exit(f"more than {int(TOLERANCE * 100)}% worse than the baseline: {', '.join(worse)}")


if __name__ == "__main__":
    main()
