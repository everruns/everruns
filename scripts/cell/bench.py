#!/usr/bin/env python3
"""Step 6's cost per turn of one cell with separate executors, against budgets.

  bench.py --sessions 200 --turns 10 --agent chat --executors 2

One cell of three nodes and N executors on this machine, a fixed load from
`cell-drive`, then what each turn cost:

- CPU per turn on the cell's leader, on its followers and on the executors
  (from /proc, so it counts everything the process did);
- the executors' calls to the cell per turn, and of them the round trips a
  turn waits for: proposals with a waiting caller (background appends are
  not waited) and reads the mirror could not answer (the executor's
  long-polls, pulls and live deltas are not waited). A one-turn load runs
  first, so the per-turn figure is the marginal one and what a new session
  costs once is reported apart;
- Raft entries and fsyncs per turn on the leader;
- the framework alone on the same load (`cell-baseline`: Engine on
  `MemoryHosting`, no Raft, no network), so the cell's own overhead is the
  difference.

Budgets (design.md, "Scale estimate" and plan step 1):

- leader CPU per turn: one core at the 115 turns/s a cell must hold, so
  8.7 ms; the "fits 75k" line is 16 cells at 78 turns/s each on 0.7 core;
- waited round trips per chat turn: 5 (Engine's own on its in-memory
  authority, plan step 1); other agents are reported, not gated.

Exit status 1 when a budget is missed. Build first:
`cargo build --release -p everruns-cell-testkit --bins`.
"""
import argparse, json, os, subprocess, sys, time, urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "scripts", "cell"))
import fleet  # noqa: E402

RELEASE = os.path.join(ROOT, "target", "release")
DRIVE = os.environ.get("CELL_DRIVE_BIN", os.path.join(RELEASE, "cell-drive"))
BASELINE = os.environ.get("CELL_BASELINE_BIN", os.path.join(RELEASE, "cell-baseline"))

LEADER_CPU_MS = 1000.0 / 115
ROUND_TRIPS = 5.0
TICK = os.sysconf("SC_CLK_TCK")


def cpu_secs(pid):
    with open(f"/proc/{pid}/stat") as f:
        rest = f.read().rsplit(")", 1)[1].split()
    # utime and stime are fields 14 and 15; `rest` starts at field 3.
    return (int(rest[11]) + int(rest[12])) / TICK


def pid_of(path):
    with open(path) as f:
        return int(f.read())


def calls(executor):
    """The executor's successful calls to cells, by kind."""
    text = urllib.request.urlopen(f"http://127.0.0.1:{7000 + executor}/metrics", timeout=5).read()
    out = {}
    for line in text.decode().splitlines():
        if line.startswith("executor_cell_calls_total{"):
            labels, value = line.rsplit(" ", 1)
            for label in ("call", "waited"):
                if f'{label}="' in labels:
                    kind = label + ":" + labels.split(f'{label}="', 1)[1].split('"', 1)[0]
                    out[kind] = out.get(kind, 0) + float(value)
    return out


def sample(executors):
    leader = fleet.leader(0)
    s = fleet.stats(0, leader, timeout=5)
    return {
        "leader": leader,
        "cpu": {n: cpu_secs(pid_of(fleet.pidfile(0, n))) for n in (1, 2, 3)},
        "executor_cpu": {e: cpu_secs(pid_of(fleet.executor_pidfile(e))) for e in range(1, executors + 1)},
        "calls": {e: calls(e) for e in range(1, executors + 1)},
        "wal": s["wal"],
    }


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--sessions", type=int, default=200)
    p.add_argument("--turns", type=int, default=10)
    p.add_argument("--concurrency", type=int, default=200)
    p.add_argument("--agent", default="chat")
    p.add_argument("--executors", type=int, default=2)
    p.add_argument("--work-ms", default="0")
    p.add_argument("--no-baseline", action="store_true")
    a = p.parse_args()
    if a.executors < 1:
        sys.exit("the bench measures separate executors: --executors 1 or more")

    subprocess.run([sys.executable, os.path.join(ROOT, "scripts/cell/fleet.py"), "down"])
    time.sleep(0.5)
    subprocess.run(["rm", "-rf", fleet.STATE])
    os.environ["CELL_WORK_MS"] = a.work_ms
    fleet.up(1, 3, 7100, executors=a.executors)
    time.sleep(2)
    gateways = ",".join(f"http://{fleet.addr(0, n)}" for n in (1, 2, 3))

    def load(turns):
        out = subprocess.run(
            [DRIVE, "load", "--gateways", gateways, "--agent", a.agent, "--sessions", str(a.sessions),
             "--turns", str(turns), "--concurrency", str(a.concurrency)],
            capture_output=True, text=True, check=True)
        return json.loads(out.stdout)

    try:
        # A one-turn load first: what a new session costs once (its record
        # and first writes) plus one turn, so the per-turn figure below is
        # the marginal one, whatever the turns per session.
        start = sample(a.executors)
        load(1)
        before = sample(a.executors)
        report = load(a.turns)
        after = sample(a.executors)
    finally:
        subprocess.run([sys.executable, os.path.join(ROOT, "scripts/cell/fleet.py"), "down"])

    turns = report["turns_completed"]
    if turns == 0 or report["errors"]:
        sys.exit(f"the load failed: {report}")
    if before["leader"] != after["leader"]:
        print(f"warning: the leader moved from {before['leader']} to {after['leader']}", file=sys.stderr)
    leader = after["leader"]
    per_turn = lambda secs: round(secs * 1000 / turns, 3)  # noqa: E731
    node_cpu = {n: after["cpu"][n] - before["cpu"][n] for n in (1, 2, 3)}
    executor_cpu = sum(after["executor_cpu"][e] - before["executor_cpu"][e] for e in after["executor_cpu"])
    kinds = {}
    for e in after["calls"]:
        for kind, value in after["calls"][e].items():
            kinds[kind] = kinds.get(kind, 0) + value - before["calls"][e].get(kind, 0)
    def waited_between(x, y):
        return sum(y["calls"][e].get(k, 0) - x["calls"][e].get(k, 0)
                   for e in y["calls"] for k in ("waited:propose", "waited:view"))
    first = waited_between(start, before) / a.sessions
    per_session = waited_between(before, after) / a.sessions
    marginal = (per_session - first) / (a.turns - 1) if a.turns > 1 else per_session
    waited = waited_between(before, after)
    result = {
        "agent": a.agent,
        "sessions": a.sessions,
        "turns": turns,
        "executors": a.executors,
        "turns_per_sec": round(report["turns_per_sec"], 1),
        "turn_p50_ms": report["turn"]["p50_ms"],
        "turn_p99_ms": report["turn"]["p99_ms"],
        "leader_cpu_ms_per_turn": per_turn(node_cpu[leader]),
        "follower_cpu_ms_per_turn": per_turn(max(node_cpu[n] for n in (1, 2, 3) if n != leader)),
        "executor_cpu_ms_per_turn": per_turn(executor_cpu),
        "calls_per_turn": {k: round(v / turns, 2) for k, v in sorted(kinds.items())},
        "waited_round_trips_per_turn_all": round(waited / turns, 2),
        "waited_round_trips_per_turn": round(marginal, 2),
        "waited_round_trips_once_per_session": round(first - marginal, 2),
        "raft_entries_per_turn": round((after["wal"]["entries"] - before["wal"]["entries"]) / turns, 2),
        "fsyncs_per_turn": round((after["wal"]["fsyncs"] - before["wal"]["fsyncs"]) / turns, 2),
        # Leader CPU over the run's wall time: how much of a core it used.
        "leader_cores": round(node_cpu[leader] / (turns / report["turns_per_sec"]), 2),
    }
    if not a.no_baseline:
        base = subprocess.run(
            [BASELINE, "--agent", a.agent, "--sessions", str(a.sessions), "--turns", str(a.turns)],
            capture_output=True, text=True, check=True, env=dict(os.environ))
        framework = json.loads(base.stdout)
        result["framework_cpu_ms_per_turn"] = round(framework["cpu_ms_per_turn"], 3)
        cell = result["leader_cpu_ms_per_turn"] + 2 * result["follower_cpu_ms_per_turn"] + result["executor_cpu_ms_per_turn"]
        result["cell_overhead_cpu_ms_per_turn"] = round(cell - framework["cpu_ms_per_turn"], 3)
    budgets = {
        "leader_cpu_ms_per_turn": (result["leader_cpu_ms_per_turn"], round(LEADER_CPU_MS, 1)),
    }
    if a.agent == "chat":
        # The round-trip budget is per chat turn; a tool turn takes more
        # model steps, each with its own commits, and is reported only.
        budgets["waited_round_trips_per_turn"] = (result["waited_round_trips_per_turn"], ROUND_TRIPS)
    # Compared at one decimal: a turn that outlives the renewal interval
    # renews once more, which shows as a few hundredths.
    result["budgets"] = {k: {"value": v, "budget": b, "ok": round(v, 1) <= b} for k, (v, b) in budgets.items()}
    print(json.dumps(result, indent=2))
    if not all(b["ok"] for b in result["budgets"].values()):
        sys.exit(1)


if __name__ == "__main__":
    main()
