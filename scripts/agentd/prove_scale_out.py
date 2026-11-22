#!/usr/bin/env python3
"""Does throughput grow with shards? One machine, each shard on its own cores.

  prove_scale_out.py [--max-shards 3] [--sessions-per-shard 400] [--turns 4]

There is one machine here, so "a machine per shard" is simulated by CPU
pinning: shard c's three nodes run on core c only (`taskset`), and the load
driver runs on the last core. For 1, 2, ... shards it runs the same burst per
shard (`worker` turns: a model call, a tool step, an answer) and records
turns per second. If shards share no bottleneck, throughput grows in step
with the number of shards; the disk (WAL fsyncs) and the object store are
still shared, as they would not be across machines.

Writes .shard/scale_out.json.
"""
import argparse, json, os, subprocess, sys, time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "scripts", "shard"))
import fleet  # noqa: E402

DRIVE = os.path.join(ROOT, "target", "release", "agentd-drive")


def pin(pid, cpus):
    subprocess.run(["taskset", "-a", "-cp", cpus, str(pid)], capture_output=True, check=True)


def run(shards, a):
    subprocess.run([sys.executable, os.path.join(ROOT, "scripts/agentd/fleet.py"), "down"])
    time.sleep(0.5)
    subprocess.run(["rm", "-rf", fleet.STATE])
    os.makedirs(fleet.STATE, exist_ok=True)
    fleet.up(shards, 3, 7100)
    for c in range(shards):
        for n in fleet.nodes_of(c):
            with open(fleet.pidfile(c, n)) as f:
                pin(int(f.read()), str(c))
    time.sleep(2)
    gateways = ",".join(f"http://{fleet.addr(c, n)}" for c in range(shards) for n in (1, 2, 3))
    sessions = a.sessions_per_shard * shards
    out = subprocess.run(
        ["taskset", "-c", str(a.driver_cpu), DRIVE, "load", "--gateways", gateways,
         "--agent", "worker", "--sessions", str(sessions), "--turns", str(a.turns),
         "--concurrency", str(sessions)],
        capture_output=True, text=True)
    report = json.loads(out.stdout)
    return {"shards": shards, "sessions": sessions, "turns": report["turns_completed"],
            "errors": report["errors"], "turns_per_sec": round(report["turns_per_sec"], 1),
            "turn_p50_ms": round(report["turn"]["p50_ms"]), "turn_p99_ms": round(report["turn"]["p99_ms"])}


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--max-shards", type=int, default=3)
    p.add_argument("--sessions-per-shard", type=int, default=400)
    p.add_argument("--turns", type=int, default=4)
    p.add_argument("--driver-cpu", type=int, default=os.cpu_count() - 1)
    p.add_argument("--work-ms", default="50", help="AGENTD_WORK_MS: the tool step's duration")
    a = p.parse_args()
    if a.max_shards > a.driver_cpu:
        sys.exit(f"{a.max_shards} shards need {a.max_shards + 1} cores (one per shard, one for the driver)")
    os.environ["AGENTD_WORK_MS"] = a.work_ms
    rows = []
    for shards in range(1, a.max_shards + 1):
        row = run(shards, a)
        rows.append(row)
        print(json.dumps(row), flush=True)
    base = rows[0]["turns_per_sec"] or 1
    for row in rows:
        row["speedup"] = round(row["turns_per_sec"] / base, 2)
    subprocess.run([sys.executable, os.path.join(ROOT, "scripts/agentd/fleet.py"), "down"])
    with open(os.path.join(fleet.STATE, "scale_out.json"), "w") as f:
        json.dump(rows, f, indent=1)
    print(json.dumps(rows, indent=2))
    sys.exit(0 if all(r["errors"] == 0 for r in rows) else 1)


if __name__ == "__main__":
    main()
