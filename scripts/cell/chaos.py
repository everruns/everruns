#!/usr/bin/env python3
"""Chaos run: kill -9 random nodes while sessions run multi-step turns.

  chaos.py --cells 1 --sessions 300 --turns 3 --kills 15
  chaos.py --cells 2 --sessions 300 --turns 3 --kills 15 --moves 100
  chaos.py --cells 2 --executors 3 --executor-kills 10   # turns on executors

One node per cell is down at a time (a 3-node cell keeps its quorum). With
--moves, sessions are also moved live between cells while they run. After
the load finishes, every session's log is checked: all accepted messages
completed with no client resend, sequences are dense, and the work tool never
ran more times than it has durable `tool.started` events.

With --executors, turns run on executors outside the cells, and
--executor-kills kills -9 a random executor that many times (it restarts
after a pause): its turns resume on another executor from the journal.
"""
import argparse, json, os, random, subprocess, sys, threading, time, urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "scripts", "cell"))
import fleet  # noqa: E402

DRIVE = os.environ.get("CELL_DRIVE_BIN", os.path.join(ROOT, "target", "release", "cell-drive"))


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--cells", type=int, default=1)
    p.add_argument("--sessions", type=int, default=300)
    p.add_argument("--turns", type=int, default=3)
    p.add_argument("--kills", type=int, default=15)
    p.add_argument("--agent", default="worker3")
    p.add_argument("--work-ms", default="200")
    p.add_argument("--moves", type=int, default=0)
    p.add_argument("--executors", type=int, default=0)
    p.add_argument("--executor-kills", type=int, default=0)
    a = p.parse_args()

    subprocess.run([sys.executable, os.path.join(ROOT, "scripts/cell/fleet.py"), "down"])
    time.sleep(0.5)
    subprocess.run(["rm", "-rf", fleet.STATE])
    ledger = os.path.join(fleet.STATE, "ledger.txt")
    os.makedirs(fleet.STATE, exist_ok=True)
    os.environ["CELL_LEDGER"] = ledger
    os.environ["CELL_WORK_MS"] = a.work_ms
    fleet.up(a.cells, 3, 7100, a.executors)
    time.sleep(1)

    gateways = ",".join(f"http://{fleet.addr(c, n)}" for c in range(a.cells) for n in (1, 2, 3))
    sessions_file = os.path.join(fleet.STATE, "sessions.txt")
    load_out = os.path.join(fleet.STATE, "load.json")
    load = subprocess.Popen(
        [DRIVE, "load", "--gateways", gateways, "--agent", a.agent, "--sessions", str(a.sessions),
         "--turns", str(a.turns), "--concurrency", str(a.sessions), "--sessions-file", sessions_file,
         "--persistent"], stdout=open(load_out, "w"))

    kills = []

    def chaos():
        time.sleep(1.5)
        for _ in range(a.kills):
            if load.poll() is not None:
                return
            cell = random.randrange(a.cells)
            node = random.choice((1, 2, 3))
            was_leader = fleet.leader(cell) == node
            fleet.kill(cell, node)
            kills.append({"cell": cell, "node": node, "leader": was_leader, "t": time.time()})
            print(f"kill -9 cell {cell} node {node} (leader={was_leader})", file=sys.stderr, flush=True)
            time.sleep(random.uniform(0.5, 2.0))
            fleet.start(cell, node)
            time.sleep(random.uniform(1.0, 2.5))

    executor_kills = []

    def executor_chaos():
        time.sleep(1.0)
        for _ in range(a.executor_kills):
            if load.poll() is not None:
                return
            n = random.randint(1, a.executors)
            fleet.kill_executor(n)
            executor_kills.append({"executor": n, "t": time.time()})
            print(f"kill -9 executor {n}", file=sys.stderr, flush=True)
            time.sleep(random.uniform(0.5, 2.0))
            fleet.start_executor(n)
            time.sleep(random.uniform(0.5, 1.5))

    moves = []

    def move_one(session, to):
        # A move is idempotent step by step, so a gateway dying mid-move is
        # retried through another one.
        for _ in range(50):
            cell = random.randrange(a.cells)
            url = f"http://{fleet.addr(cell, random.choice((1, 2, 3)))}/v1/sessions/{session}/move"
            req = urllib.request.Request(url, data=json.dumps({"cell": to}).encode(), method="POST",
                                         headers={"content-type": "application/json"})
            try:
                with urllib.request.urlopen(req, timeout=120) as r:
                    return json.load(r)
            except Exception:
                time.sleep(0.2)
        return None

    def mover():
        while not os.path.exists(sessions_file):
            time.sleep(0.1)
        time.sleep(0.5)
        ids = [l for l in open(sessions_file).read().split("\n") if l]
        for _ in range(a.moves // 4):
            if load.poll() is not None:
                return
            session = random.choice(ids)
            to = random.randrange(a.cells)
            result = move_one(session, to)
            if result and result.get("moved"):
                moves.append(result)

    threads = [threading.Thread(target=chaos)]
    if a.executors and a.executor_kills:
        threads.append(threading.Thread(target=executor_chaos))
    if a.moves and a.cells > 1:
        threads += [threading.Thread(target=mover) for _ in range(4)]
    for t in threads:
        t.start()
    started = time.time()
    while load.poll() is None:
        time.sleep(5)
        done = 0
        for cell in range(a.cells):
            for node in (1, 2, 3):
                try:
                    done += fleet.stats(cell, node)["turns_completed"]
                    break
                except Exception:
                    pass
        print(f"{int(time.time() - started)}s: {done} / {a.sessions * a.turns} turns completed",
              flush=True)
    for t in threads:
        t.join()
    fleet.wait_ready()
    time.sleep(1)

    check = subprocess.run(
        [DRIVE, "check", "--gateways", gateways, "--sessions-file", sessions_file,
         "--turns", str(a.turns), "--ledger", ledger], capture_output=True, text=True)
    load_report = json.load(open(load_out))
    check_report = json.loads(check.stdout) if check.stdout.strip() else {"ok": False, "stderr": check.stderr}
    summary = {
        "kills": len(kills),
        "leader_kills": sum(1 for k in kills if k["leader"]),
        "executor_kills": len(executor_kills),
        "moves": len(moves),
        "move_p50_ms": sorted(m["elapsed_ms"] for m in moves)[len(moves) // 2] if moves else None,
        "sessions": a.sessions,
        "turns": load_report["turns_completed"],
        "load_errors": load_report["errors"],
        "wall_secs": round(load_report["wall_secs"], 2),
        "turn_p50_ms": load_report["turn"]["p50_ms"],
        "turn_p99_ms": load_report["turn"]["p99_ms"],
        "check": check_report,
    }
    print(json.dumps(summary, indent=2))
    sys.exit(0 if check_report.get("ok") and load_report["errors"] == 0 else 1)


if __name__ == "__main__":
    main()
