#!/usr/bin/env python3
"""Start, stop and break a local cell fleet.

  fleet.py up --cells 2 --nodes 3        # topology + node processes
  fleet.py kill <cell> <node>            # kill -9 one node
  fleet.py start <cell> <node>           # (re)start one node
  fleet.py leader <cell>                 # print the cell's leader id
  fleet.py join <cell> <node>            # start a node and add it to a running cell
  fleet.py add-cell <cell>               # start a new cell; it registers itself
  fleet.py up --executors 2              # nodes leave turns to 2 executors
  fleet.py executor-kill <n>             # kill -9 executor n
  fleet.py executor-start <n>            # (re)start executor n
  fleet.py down                          # stop everything

With executors, every node runs with `--executor-cap` (CELL_EXECUTOR_CAP,
default 256 here) and turns run only on the executors, which pull them
from every cell with credits. Executor n serves /metrics on port 7000 + n.

State lives in .cell/ (topology.json, data/, logs/, pids/). topology.json
is only the seed the fleet starts from: nodes and cells added later are not
written to it (the runtime learns them from Raft membership and the
directory's cell registry); this script keeps its own note of them in
extra.json so it can address and restart them.
"""
import argparse, json, os, signal, subprocess, sys, time, urllib.request

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
STATE = os.path.join(ROOT, ".cell")
TOPO = os.path.join(STATE, "topology.json")


def binary():
    return os.environ.get("CELL_NODE_BIN", os.path.join(ROOT, "target", "release", "cell-testnode"))


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


def save_extra(cell, node, entry):
    e = extra()
    e.setdefault(str(cell), {})[str(node)] = entry
    with open(EXTRA, "w") as f:
        json.dump(e, f, indent=2)


def nodes_of(cell):
    out = dict(topology()["cells"].get(str(cell), {}))
    out.update({n: v["addr"] for n, v in extra().get(str(cell), {}).items()})
    return out


def cells():
    return sorted(set(topology()["cells"]) | set(extra()), key=int)


def addr(cell, node):
    return nodes_of(cell)[str(node)]


def port(cell, node, base=7100):
    return f"127.0.0.1:{base + 100 * int(cell) + int(node)}"


def pidfile(cell, node):
    return os.path.join(STATE, "pids", f"c{cell}n{node}.pid")


def start(cell, node, extra_env=None):
    os.makedirs(os.path.join(STATE, "logs"), exist_ok=True)
    os.makedirs(os.path.join(STATE, "pids"), exist_ok=True)
    log = open(os.path.join(STATE, "logs", f"c{cell}n{node}.log"), "a")
    env = dict(os.environ)
    # Agents that call the runtime's own API (hand-offs) use their node's.
    env["CELL_GATEWAY"] = addr(cell, node)
    env.update(extra_env or {})
    entry = extra().get(str(cell), {}).get(str(node), {})
    args = [binary(), "node", "--topology", entry.get("topology", TOPO),
            "--cell", str(cell), "--node", str(node),
            "--data", os.path.join(STATE, "data", f"c{cell}n{node}"),
            "--store", os.environ.get("CELL_STORE", "file://" + os.path.join(STATE, "store"))]
    if entry.get("join"):
        args += ["--join", "--listen", entry["addr"]]
    args += os.environ.get("CELL_NODE_ARGS", "").split()
    proc = subprocess.Popen(args, stdout=log, stderr=log, env=env, start_new_session=True)
    with open(pidfile(cell, node), "w") as f:
        f.write(str(proc.pid))
    return proc.pid


EXECUTOR_IDS = 1 << 32


def executor_pidfile(n):
    return os.path.join(STATE, "pids", f"e{n}.pid")


def start_executor(n):
    """Start executor n: it pulls turns from every cell in the topology."""
    os.makedirs(os.path.join(STATE, "logs"), exist_ok=True)
    os.makedirs(os.path.join(STATE, "pids"), exist_ok=True)
    log = open(os.path.join(STATE, "logs", f"e{n}.log"), "a")
    args = [binary(), "executor", "--topology", TOPO, "--id", str(EXECUTOR_IDS + int(n)),
            "--store", os.environ.get("CELL_STORE", "file://" + os.path.join(STATE, "store")),
            "--listen", f"127.0.0.1:{7000 + int(n)}"]
    args += os.environ.get("CELL_EXECUTOR_ARGS", "").split()
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


def kill(cell, node, sig=signal.SIGKILL):
    try:
        with open(pidfile(cell, node)) as f:
            pid = int(f.read())
        os.kill(pid, sig)
        os.remove(pidfile(cell, node))
        return pid
    except (FileNotFoundError, ProcessLookupError, ValueError):
        # No node, a dead one, or a pidfile cut short by a kill mid-write.
        try:
            os.remove(pidfile(cell, node))
        except FileNotFoundError:
            pass
        return None


def stats(cell, node, timeout=1.0):
    url = f"http://{addr(cell, node)}/cell/stats"
    with urllib.request.urlopen(url, timeout=timeout) as r:
        return json.load(r)


def leader(cell):
    for node in nodes_of(cell):
        try:
            s = stats(cell, node)
            if s.get("leader") is not None:
                return s["leader"]
        except Exception:
            pass
    return None


def wait_ready(timeout=30):
    deadline = time.time() + timeout
    for cell in cells():
        while leader(cell) is None:
            if time.time() > deadline:
                sys.exit(f"cell {cell} has no leader")
            time.sleep(0.2)


def up(cells, nodes, base, executors=0):
    # A node still running from an earlier fleet would hold its port, and
    # the new pidfile would hide it from `down`.
    if os.path.isdir(os.path.join(STATE, "pids")) and os.listdir(os.path.join(STATE, "pids")):
        down()
        time.sleep(1)
    os.makedirs(STATE, exist_ok=True)
    if os.path.exists(EXTRA):
        os.remove(EXTRA)
    topo = {"directory_cell": 0, "cells": {
        str(c): {str(n): f"127.0.0.1:{base + 100 * c + n}" for n in range(1, nodes + 1)}
        for c in range(cells)}}
    with open(TOPO, "w") as f:
        json.dump(topo, f, indent=2)
    node_env = {}
    if executors:
        node_env["CELL_EXECUTOR_CAP"] = os.environ.get("CELL_EXECUTOR_CAP", "256")
    for c in range(cells):
        for n in range(1, nodes + 1):
            start(c, n, node_env)
    wait_ready()
    for n in range(1, executors + 1):
        start_executor(n)
    return topo


def post(path, body, timeout=120):
    """POST to the first directory-cell node that answers."""
    last = None
    for node in nodes_of(topology()["directory_cell"]):
        req = urllib.request.Request(f"http://{addr(topology()['directory_cell'], node)}{path}",
                                     data=json.dumps(body).encode(), method="POST",
                                     headers={"content-type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                return json.load(r)
        except Exception as e:
            last = e
    raise SystemExit(f"POST {path}: {last}")


def join(cell, node):
    """Start an empty node and ask the cell's leader to add it."""
    a = port(cell, node)
    save_extra(cell, node, {"addr": a, "join": True})
    start(cell, node)
    return post(f"/v1/cells/{cell}/nodes", {"node": int(node), "addr": a})


def add_cell(cell, nodes=3):
    """Start a new cell from a seed naming only the directory cell and
    itself. Gateways learn of it from the directory's registry."""
    seed = {"directory_cell": topology()["directory_cell"], "cells": {
        str(topology()["directory_cell"]): topology()["cells"][str(topology()["directory_cell"])],
        str(cell): {str(n): port(cell, n) for n in range(1, nodes + 1)}}}
    path = os.path.join(STATE, f"seed-c{cell}.json")
    with open(path, "w") as f:
        json.dump(seed, f, indent=2)
    for n in range(1, nodes + 1):
        save_extra(cell, n, {"addr": port(cell, n), "topology": path})
        start(cell, n)
    wait_ready()
    return seed


def down():
    pids = os.path.join(STATE, "pids")
    if os.path.isdir(pids):
        for name in os.listdir(pids):
            if name.startswith("e"):
                kill_executor(int(name[1:-4]), signal.SIGTERM)
                continue
            cell, node = name[1:-4].split("n")
            kill(int(cell), int(node), signal.SIGTERM)


def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="cmd", required=True)
    u = sub.add_parser("up"); u.add_argument("--cells", type=int, default=1); u.add_argument("--nodes", type=int, default=3); u.add_argument("--base", type=int, default=7100); u.add_argument("--executors", type=int, default=0)
    sub.add_parser("down")
    for name in ("kill", "start"):
        k = sub.add_parser(name); k.add_argument("cell", type=int); k.add_argument("node", type=int)
    for name in ("executor-kill", "executor-start"):
        e = sub.add_parser(name); e.add_argument("n", type=int)
    l = sub.add_parser("leader"); l.add_argument("cell", type=int)
    sub.add_parser("ready")
    j = sub.add_parser("join"); j.add_argument("cell", type=int); j.add_argument("node", type=int)
    c = sub.add_parser("add-cell"); c.add_argument("cell", type=int); c.add_argument("--nodes", type=int, default=3)
    a = p.parse_args()
    if a.cmd == "up": print(json.dumps(up(a.cells, a.nodes, a.base, a.executors)))
    elif a.cmd == "down": down()
    elif a.cmd == "kill": print(kill(a.cell, a.node))
    elif a.cmd == "start": print(start(a.cell, a.node))
    elif a.cmd == "executor-kill": print(kill_executor(a.n))
    elif a.cmd == "executor-start": print(start_executor(a.n))
    elif a.cmd == "leader": print(leader(a.cell))
    elif a.cmd == "ready": wait_ready()
    elif a.cmd == "join": print(json.dumps(join(a.cell, a.node)))
    elif a.cmd == "add-cell": print(json.dumps(add_cell(a.cell, a.nodes)))


if __name__ == "__main__":
    main()
