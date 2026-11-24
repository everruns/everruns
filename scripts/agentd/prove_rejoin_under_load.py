#!/usr/bin/env python3
"""A follower killed under load rejoins without hurting its shard.

  prove_rejoin_under_load.py [--rate 20] [--secs 150] [--down 90]

Starts one shard with two executors, drives chat sessions at `--rate` new
sessions a second for `--secs` (each holding its stream one time in five),
kills one follower 60 s in (clients use the other members' gateways) and starts it again `--down` seconds later, by
when the leader has purged the entries the follower missed, so it catches
up by snapshot. Found by EKS run 43803384547: such a follower answered
"not found" for sessions created while it was down (6,790 sessions lost
their first message), and its elections kept restarting its own catch-up.

Pass: the driver sees no errors, the leader keeps its term from the
restart to the end (no elections), and the follower catches up.
"""
import argparse, json, os, signal, subprocess, sys, time, urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "scripts", "shard"))
import fleet  # noqa: E402

DRIVE = os.environ.get("AGENTD_DRIVE_BIN", os.path.join(ROOT, "target", "release", "agentd-drive"))
KILL_AT = 60


def metric(node, name):
    try:
        url = f"http://{fleet.addr(0, node)}/metrics"
        text = urllib.request.urlopen(url, timeout=2).read().decode()
    except Exception:
        return None
    for line in text.splitlines():
        if line.startswith(name + " ") or line.startswith(name + "{"):
            return float(line.split()[-1])
    return None


def rss_mb(node):
    try:
        with open(fleet.pidfile(0, node)) as f:
            pid = int(f.read())
        for line in open(f"/proc/{pid}/status"):
            if line.startswith("VmRSS"):
                return int(line.split()[1]) // 1024
    except Exception:
        return None


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--rate", type=float, default=20)
    p.add_argument("--secs", type=int, default=150)
    p.add_argument("--down", type=int, default=90)
    p.add_argument("--turns", type=int, default=10)
    p.add_argument("--think", type=float, default=10)
    p.add_argument("--seed-per-shard", type=int, default=0, help="sessions each shard holds before the fault (the verifier: goal density)")
    a = p.parse_args()

    subprocess.run([sys.executable, os.path.join(ROOT, "scripts/agentd/fleet.py"), "down"])
    time.sleep(0.5)
    subprocess.run(["rm", "-rf", fleet.STATE])
    os.environ["AGENTD_NODE_ARGS"] = "--snapshot-every 2000 --keep-log 256"
    os.environ.setdefault("AGENTD_MEMORY_BUDGET_MB", "3000")
    fleet.up(1, 3, 7100, 2)
    seeded = fleet.seed(a.seed_per_shard)
    time.sleep(2)
    leader = fleet.leader(0)
    follower = next(n for n in (1, 2, 3) if n != leader)
    # Clients reach the shard through the other members' gateways: the
    # follower's own gateway is down with it, which says nothing.
    gateways = ",".join(f"http://{fleet.addr(0, n)}" for n in (1, 2, 3) if n != follower)
    drive = subprocess.Popen(
        [DRIVE, "load", "--gateways", gateways, "--agent", "sim", "--scenarios", "chat",
         "--sessions", "0", "--rate", str(a.rate), "--duration", str(a.secs),
         "--turns", str(a.turns), "--think", str(a.think), "--stream", "0.2"],
        stdout=subprocess.PIPE, stderr=open(os.path.join(fleet.STATE, "drive.log"), "w"), text=True)
    started = time.time()
    timeline = []
    killed_at = restarted_at = None
    term_at_restart = None
    terms_after = set()
    peak_leader_rss = 0
    while drive.poll() is None:
        t = round(time.time() - started)
        if killed_at is None and t >= KILL_AT:
            leader = fleet.leader(0)
            fleet.kill(0, follower)
            killed_at = t
            print(f"t={t}: killed follower {follower} (leader {leader})", flush=True)
        if killed_at is not None and restarted_at is None and t >= killed_at + a.down:
            term_at_restart = metric(leader, "agentd_shard_raft_term")
            fleet.start(0, follower)
            restarted_at = t
            print(f"t={t}: started follower {follower} again", flush=True)
        row = {"t": t}
        for n in (1, 2, 3):
            row[n] = {"applied": metric(n, "agentd_shard_raft_applied_index"),
                      "term": metric(n, "agentd_shard_raft_term"),
                      "behind": metric(n, "agentd_shard_catching_up"), "rss": rss_mb(n)}
        timeline.append(row)
        if restarted_at is not None and row[leader]["term"] is not None:
            terms_after.add(row[leader]["term"])
        peak_leader_rss = max(peak_leader_rss, row[leader]["rss"] or 0)
        print(json.dumps(row), flush=True)
        time.sleep(5)
    report = json.loads(drive.stdout.read() or "{}")
    lead_applied = metric(leader, "agentd_shard_raft_applied_index") or 0
    follower_applied = metric(follower, "agentd_shard_raft_applied_index") or 0
    result = {
        "sessions_per_shard": seeded,
        "sessions": report.get("sessions"),
        "turns_completed": report.get("turns_completed"),
        "errors": report.get("errors"),
        "first_errors": report.get("first_errors", [])[:5],
        "stream_errors": report.get("stream_errors"),
        "leader": leader,
        "follower": follower,
        "term_at_restart": term_at_restart,
        "leader_terms_after_restart": sorted(terms_after),
        "follower_gap_at_end": lead_applied - follower_applied,
        "peak_leader_rss_mb": peak_leader_rss,
        "caught_up_secs": next(
            (r["t"] - restarted_at for r in timeline
             if restarted_at is not None and r["t"] > restarted_at
             and r[follower]["applied"] is not None and r[leader]["applied"] is not None
             and r[leader]["applied"] - r[follower]["applied"] <= 256), None),
    }
    result["ok"] = (result["errors"] == 0 and len(terms_after) <= 1
                    and result["caught_up_secs"] is not None)
    print(json.dumps(result, indent=2))
    sys.exit(0 if result["ok"] else 1)


if __name__ == "__main__":
    signal.signal(signal.SIGTERM, lambda *_: sys.exit(124))
    try:
        main()
    finally:
        subprocess.run([sys.executable, os.path.join(ROOT, "scripts/agentd/fleet.py"), "down"])
