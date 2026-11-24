#!/usr/bin/env python3
"""Start, stop and break a local shard fleet.

  fleet.py up --shards 2 --nodes 3        # topology + node processes
  fleet.py kill <shard> <node>            # kill -9 one node
  fleet.py start <shard> <node>           # (re)start one node
  fleet.py leader <shard>                 # print the shard's leader id
  fleet.py join <shard> <node>            # start a node and add it to a running shard
  fleet.py add-shard <shard>               # start a new shard; it registers itself
  fleet.py up --executors 2              # nodes leave turns to 2 executors
  fleet.py executor-kill <n>             # kill -9 executor n
  fleet.py executor-start <n>            # (re)start executor n
  fleet.py down                          # stop everything

With executors, every node runs with `--executor-cap` (AGENTD_EXECUTOR_CAP,
default 256 here) and turns run only on the executors, which pull them
from every shard with credits. Executor n serves /metrics on port 7000 + n.

State lives in .shard/ (topology.json, data/, logs/, pids/). topology.json
is only the seed the fleet starts from: nodes and shards added later are not
written to it (the runtime learns them from Raft membership and the
directory's shard registry); this script keeps its own note of them in
extra.json so it can address and restart them.
"""
import argparse, json, os, signal, subprocess, sys, time, urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
STATE = os.path.join(ROOT, ".shard")
TOPO = os.path.join(STATE, "topology.json")


def binary():
    return os.environ.get("AGENTD_NODE_BIN", os.path.join(ROOT, "target", "release", "agentd-testnode"))


def topology():
    with open(TOPO) as f:
        return json.load(f)


EXTRA = os.path.join(STATE, "extra.json")


def extra():
    try:
        with open(EXTRA) as f:
            return json.load(f)
    except FileNotFoundError:
        return {}


def save_extra(shard, node, entry):
    e = extra()
    e.setdefault(str(shard), {})[str(node)] = entry
    with open(EXTRA, "w") as f:
        json.dump(e, f, indent=2)


def nodes_of(shard):
    out = dict(topology()["shards"].get(str(shard), {}))
    out.update({n: v["addr"] for n, v in extra().get(str(shard), {}).items()})
    return out


def shards():
    return sorted(set(topology()["shards"]) | set(extra()), key=int)


def addr(shard, node):
    return nodes_of(shard)[str(node)]


def port(shard, node, base=7100):
    return f"127.0.0.1:{base + 100 * int(shard) + int(node)}"


def pidfile(shard, node):
    return os.path.join(STATE, "pids", f"c{shard}n{node}.pid")


def start(shard, node, extra_env=None):
    os.makedirs(os.path.join(STATE, "logs"), exist_ok=True)
    os.makedirs(os.path.join(STATE, "pids"), exist_ok=True)
    log = open(os.path.join(STATE, "logs", f"c{shard}n{node}.log"), "a")
    env = dict(os.environ)
    # Agents that call the runtime's own API (hand-offs) use their node's.
    env["AGENTD_GATEWAY"] = addr(shard, node)
    # Raft on its own listener and threads, as deployed: node port + 50
    # (a shard's nodes use ports base + 100 * shard + 1..49).
    env.setdefault("AGENTD_RAFT_PORT_OFFSET", "50")
    env.update(extra_env or {})
    entry = extra().get(str(shard), {}).get(str(node), {})
    args = [binary(), "node", "--topology", entry.get("topology", TOPO),
            "--shard", str(shard), "--node", str(node),
            "--data", os.path.join(STATE, "data", f"c{shard}n{node}"),
            "--store", os.environ.get("AGENTD_STORE", "file://" + os.path.join(STATE, "store"))]
    if entry.get("join"):
        args += ["--join", "--listen", entry["addr"]]
    args += os.environ.get("AGENTD_NODE_ARGS", "").split()
    proc = subprocess.Popen(args, stdout=log, stderr=log, env=env, start_new_session=True)
    with open(pidfile(shard, node), "w") as f:
        f.write(str(proc.pid))
    return proc.pid


EXECUTOR_IDS = 1 << 32


def executor_pidfile(n):
    return os.path.join(STATE, "pids", f"e{n}.pid")


def start_executor(n):
    """Start executor n: it pulls turns from every shard in the topology."""
    os.makedirs(os.path.join(STATE, "logs"), exist_ok=True)
    os.makedirs(os.path.join(STATE, "pids"), exist_ok=True)
    log = open(os.path.join(STATE, "logs", f"e{n}.log"), "a")
    args = [binary(), "executor", "--topology", TOPO, "--id", str(EXECUTOR_IDS + int(n)),
            "--store", os.environ.get("AGENTD_STORE", "file://" + os.path.join(STATE, "store")),
            "--listen", f"127.0.0.1:{7000 + int(n)}"]
    args += os.environ.get("AGENTD_EXECUTOR_ARGS", "").split()
    proc = subprocess.Popen(args, stdout=log, stderr=log, env=dict(os.environ),
                            start_new_session=True)
    with open(executor_pidfile(n), "w") as f:
        f.write(str(proc.pid))
    return proc.pid


def kill_executor(n, sig=signal.SIGKILL):
    try:
        with open(executor_pidfile(n)) as f:
            pid = int(f.read())
        os.kill(pid, sig)
        os.remove(executor_pidfile(n))
        return pid
    except (FileNotFoundError, ProcessLookupError, ValueError):
        try:
            os.remove(executor_pidfile(n))
        except FileNotFoundError:
            pass
        return None


def kill(shard, node, sig=signal.SIGKILL):
    try:
        with open(pidfile(shard, node)) as f:
            pid = int(f.read())
        os.kill(pid, sig)
        os.remove(pidfile(shard, node))
        return pid
    except (FileNotFoundError, ProcessLookupError, ValueError):
        # No node, a dead one, or a pidfile cut short by a kill mid-write.
        try:
            os.remove(pidfile(shard, node))
        except FileNotFoundError:
            pass
        return None


def stats(shard, node, timeout=1.0):
    url = f"http://{addr(shard, node)}/shard/stats"
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return json.load(r)


def leader(shard):
    for node in nodes_of(shard):
        try:
            s = stats(shard, node)
            if s.get("leader") is not None:
                return s["leader"]
        except Exception:
            pass
    return None


def wait_ready(timeout=30):
    deadline = time.time() + timeout
    for shard in shards():
        while leader(shard) is None:
            if time.time() > deadline:
                sys.exit(f"shard {shard} has no leader")
            time.sleep(0.2)


def up(shards, nodes, base, executors=0):
    # A node still running from an earlier fleet would hold its port, and
    # the new pidfile would hide it from `down`.
    if os.path.isdir(os.path.join(STATE, "pids")) and os.listdir(os.path.join(STATE, "pids")):
        down()
        time.sleep(1)
    os.makedirs(STATE, exist_ok=True)
    if os.path.exists(EXTRA):
        os.remove(EXTRA)
    topo = {"directory_shard": 0, "shards": {
        str(c): {str(n): f"127.0.0.1:{base + 100 * c + n}" for n in range(1, nodes + 1)}
        for c in range(shards)}}
    with open(TOPO, "w") as f:
        json.dump(topo, f, indent=2)
    node_env = {}
    if executors:
        node_env["AGENTD_EXECUTOR_CAP"] = os.environ.get("AGENTD_EXECUTOR_CAP", "256")
    for c in range(shards):
        for n in range(1, nodes + 1):
            start(c, n, node_env)
    wait_ready()
    for n in range(1, executors + 1):
        start_executor(n)
    return topo


def seed(per_shard, which=None):
    """Give each shard `per_shard` sessions (one finished turn each) before a
    proof injects its fault, and return the sessions each shard holds.

    Decision: the 75k verifier counts a proof's per-shard points only at the
    goal's density (25,000 sessions a shard). What a fault costs grows with
    a shard's size (snapshot size for catch-up and rejoin, log written
    before a purge for a store outage), so a proof on an empty shard proves
    less than it seems to. Proofs take `--seed-per-shard` and report
    `sessions_per_shard`; the verifier passes the goal's density.
    """
    which = [int(c) for c in shards()] if which is None else list(which)
    if per_shard <= 0:
        return {}
    gws = ",".join(f"http://{addr(c, n)}" for c in which for n in nodes_of(c))
    drive = os.environ.get("AGENTD_DRIVE_BIN", os.path.join(
        ROOT, os.environ.get("AGENTD_BIN_DIR", os.path.join("target", "release")), "agentd-drive"))
    def holding():
        held = {}
        for c in which:
            for n in nodes_of(c):
                try:
                    with urllib.request.urlopen(f"http://{addr(c, n)}/shard/inspect", timeout=5) as r:
                        held[str(c)] = max(held.get(str(c), 0), json.load(r).get("sessions") or 0)
                except Exception:
                    pass
        return held

    # Gateways place sessions unevenly across shards, so seed in rounds until
    # the emptiest shard holds `per_shard` (each round tops up its shortfall
    # times the shard count, plus a tenth), at most four rounds.
    started, errors, want = time.time(), 0, per_shard * len(which)
    held = {}
    for _ in range(4):
        out = subprocess.run([drive, "load", "--gateways", gws, "--agent", "sim", "--scenarios", "chat",
                              "--sessions", str(want), "--turns", "1",
                              "--concurrency", "500"], capture_output=True, text=True)
        errors += (json.loads(out.stdout or "{}") or {}).get("errors") or 0
        held = holding()
        short = per_shard - min((held.get(str(c), 0) for c in which), default=0)
        if short <= 0:
            break
        want = int(short * len(which) * 1.1) + 1
    print(json.dumps({"seeded": held, "seed_s": round(time.time() - started), "seed_errors": errors}), flush=True)
    return held


def post(path, body, timeout=120):
    """POST to the first directory-shard node that answers."""
    last = None
    for node in nodes_of(topology()["directory_shard"]):
        req = urllib.request.Request(f"http://{addr(topology()['directory_shard'], node)}{path}",
                                     data=json.dumps(body).encode(), method="POST",
                                     headers={"content-type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                return json.load(r)
        except Exception as e:
            last = e
    raise SystemExit(f"POST {path}: {last}")


def join(shard, node):
    """Start an empty node and ask the shard's leader to add it."""
    a = port(shard, node)
    save_extra(shard, node, {"addr": a, "join": True})
    start(shard, node)
    return post(f"/v1/shards/{shard}/nodes", {"node": int(node), "addr": a})


def add_shard(shard, nodes=3):
    """Start a new shard from a seed naming only the directory shard and
    itself. Gateways learn of it from the directory's registry."""
    seed = {"directory_shard": topology()["directory_shard"], "shards": {
        str(topology()["directory_shard"]): topology()["shards"][str(topology()["directory_shard"])],
        str(shard): {str(n): port(shard, n) for n in range(1, nodes + 1)}}}
    path = os.path.join(STATE, f"seed-c{shard}.json")
    with open(path, "w") as f:
        json.dump(seed, f, indent=2)
    for n in range(1, nodes + 1):
        save_extra(shard, n, {"addr": port(shard, n), "topology": path})
        start(shard, n)
    wait_ready()
    return seed


def down():
    pids = os.path.join(STATE, "pids")
    executors = []
    if os.path.isdir(pids):
        for name in os.listdir(pids):
            if name.startswith("e"):
                pid = kill_executor(int(name[1:-4]), signal.SIGTERM)
                if pid:
                    executors.append(pid)
                continue
            shard, node = name[1:-4].split("n")
            kill(int(shard), int(node), signal.SIGTERM)
    # A draining executor waits for its turns to hand off, and with the
    # shards gone they never do: give it a moment, then stop it, so it
    # cannot hold its port into the next fleet.
    deadline = time.time() + 5
    while executors and time.time() < deadline:
        executors = [p for p in executors if alive(p)]
        time.sleep(0.2)
    for pid in executors:
        try:
            os.kill(pid, signal.SIGKILL)
        except ProcessLookupError:
            pass


def alive(pid):
    try:
        os.kill(pid, 0)
        return True
    except ProcessLookupError:
        return False


def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="cmd", required=True)
    u = sub.add_parser("up"); u.add_argument("--shards", type=int, default=1); u.add_argument("--nodes", type=int, default=3); u.add_argument("--base", type=int, default=7100); u.add_argument("--executors", type=int, default=0)
    sub.add_parser("down")
    for name in ("kill", "start"):
        k = sub.add_parser(name); k.add_argument("shard", type=int); k.add_argument("node", type=int)
    for name in ("executor-kill", "executor-start"):
        e = sub.add_parser(name); e.add_argument("n", type=int)
    l = sub.add_parser("leader"); l.add_argument("shard", type=int)
    sub.add_parser("ready")
    j = sub.add_parser("join"); j.add_argument("shard", type=int); j.add_argument("node", type=int)
    c = sub.add_parser("add-shard"); c.add_argument("shard", type=int); c.add_argument("--nodes", type=int, default=3)
    a = p.parse_args()
    if a.cmd == "up": print(json.dumps(up(a.shards, a.nodes, a.base, a.executors)))
    elif a.cmd == "down": down()
    elif a.cmd == "kill": print(kill(a.shard, a.node))
    elif a.cmd == "start": print(start(a.shard, a.node))
    elif a.cmd == "executor-kill": print(kill_executor(a.n))
    elif a.cmd == "executor-start": print(start_executor(a.n))
    elif a.cmd == "leader": print(leader(a.shard))
    elif a.cmd == "ready": wait_ready()
    elif a.cmd == "join": print(json.dumps(join(a.shard, a.node)))
    elif a.cmd == "add-shard": print(json.dumps(add_shard(a.shard, a.nodes)))


if __name__ == "__main__":
    main()
