#!/usr/bin/env python3
"""Session files through the gateway: latency, and no lost write under chaos.

  files_bench.py [--sessions 20] [--rounds 30] [--concurrency 16]

Starts one 3-node shard (`fleet.py`), creates `--sessions` sessions, then
measures the console's file calls through the gateway of a rotating node
(so follower reads, and their read barrier, are part of every number):
upload (`PUT fs/_/upload`), read (`GET fs/{path}`), listing (`GET fs`) and
download (`GET fs/_/download`), for 1 KiB, 16 KiB, 64 KiB and 1 MiB files.

Then the chaos phase: uploads keep going while the leader and then a
follower are killed (kill -9) and restarted. Every upload the gateway
acknowledged (200) must be there afterwards, byte for byte.

Pass (step 11.1's gate): upload and download p99 under 200 ms for files up
to 1 MiB, and no acknowledged write lost.
"""
import argparse, hashlib, json, os, random, sys, threading, time, urllib.request
from concurrent.futures import ThreadPoolExecutor

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(ROOT, "scripts", "shard"))
import fleet  # noqa: E402

SIZES = [("1KiB", 1024), ("16KiB", 16 * 1024), ("64KiB", 64 * 1024), ("1MiB", 1024 * 1024)]
GATE_MS = 200.0
AGENT = {
    "name": "files",
    "instructions": "Keep files.",
    "model": {"provider": "llmsim", "model": "chat"},
}


def request(method, url, body=None, headers=None, timeout=30):
    req = urllib.request.Request(url, data=body, method=method, headers=headers or {})
    with urllib.request.urlopen(req, timeout=timeout) as r:
        return r.status, r.read()


def node_url(i):
    nodes = sorted(fleet.nodes_of(0))
    return "http://" + fleet.addr(0, nodes[i % len(nodes)])


def workspace(session):
    return "wsp_" + session.split("_", 1)[1]


def create_sessions(n):
    out = []
    for i in range(n):
        _, body = request("POST", node_url(i) + "/v1/sessions",
                          json.dumps({"agent": AGENT}).encode(),
                          {"content-type": "application/json"})
        out.append(json.loads(body)["id"])
    return out


def pct(values, p):
    values = sorted(values)
    return values[min(len(values) - 1, int(len(values) * p / 100))]


def measure(sessions, rounds, concurrency):
    timings = {}
    lock = threading.Lock()
    counter = iter(range(10**9))

    def record(key, ms):
        with lock:
            timings.setdefault(key, []).append(ms)

    def one(job):
        session, label, size, r = job
        i = next(counter)
        base = node_url(i) + f"/v1/workspaces/{workspace(session)}/fs"
        data = os.urandom(size // 2).hex().encode()[:size]
        path = f"bench/{label}/f{r}.txt"
        t = time.perf_counter()
        request("PUT", f"{base}/_/upload/{path}", data, {"content-type": "application/octet-stream"})
        record(f"upload {label}", (time.perf_counter() - t) * 1000)
        base = node_url(i + 1) + f"/v1/workspaces/{workspace(session)}/fs"
        t = time.perf_counter()
        _, got = request("GET", f"{base}/_/download/{path}")
        record(f"download {label}", (time.perf_counter() - t) * 1000)
        assert got == data, f"download of {path} differs"
        t = time.perf_counter()
        request("GET", f"{base}/{path}")
        record(f"read {label}", (time.perf_counter() - t) * 1000)
        t = time.perf_counter()
        request("GET", f"{base}/bench/{label}")
        record("list", (time.perf_counter() - t) * 1000)

    jobs = [(s, label, size, r) for r in range(rounds) for s in sessions for label, size in SIZES]
    random.shuffle(jobs)
    with ThreadPoolExecutor(concurrency) as pool:
        list(pool.map(one, jobs))
    return timings


def chaos(sessions, seconds, concurrency):
    acked = {}
    lock = threading.Lock()
    stop = time.time() + seconds
    errors = [0]

    def writer(w):
        n = 0
        while time.time() < stop:
            session = random.choice(sessions)
            data = f"writer {w} file {n} ".encode() * random.choice([1, 100, 3000])
            path = f"chaos/w{w}/f{n}.txt"
            n += 1
            try:
                status, _ = request("PUT", node_url(w + n) + f"/v1/workspaces/{workspace(session)}/fs/_/upload/{path}",
                                    data, {"content-type": "application/octet-stream"}, timeout=60)
            except Exception:
                errors[0] += 1
                continue
            if status == 200:
                with lock:
                    acked[(session, path)] = hashlib.sha256(data).hexdigest()

    threads = [threading.Thread(target=writer, args=(w,)) for w in range(concurrency)]
    for t in threads:
        t.start()
    time.sleep(seconds / 5)
    leader = fleet.leader(0)
    print(f"  kill -9 leader n{leader}", flush=True)
    fleet.kill(0, leader)
    time.sleep(seconds / 5)
    fleet.start(0, leader)
    time.sleep(seconds / 5)
    follower = next(n for n in sorted(fleet.nodes_of(0)) if int(n) != int(fleet.leader(0) or 0))
    print(f"  kill -9 follower n{follower}", flush=True)
    fleet.kill(0, follower)
    time.sleep(seconds / 10)
    fleet.start(0, follower)
    for t in threads:
        t.join()
    time.sleep(2)
    lost = 0
    for (session, path), digest in acked.items():
        _, got = request("GET", node_url(random.randrange(3)) + f"/v1/workspaces/{workspace(session)}/fs/_/download/{path}", timeout=60)
        if hashlib.sha256(got).hexdigest() != digest:
            lost += 1
    return len(acked), errors[0], lost


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--sessions", type=int, default=20)
    p.add_argument("--rounds", type=int, default=30)
    p.add_argument("--concurrency", type=int, default=16)
    p.add_argument("--chaos-seconds", type=int, default=40)
    p.add_argument("--keep", action="store_true", help="leave the fleet running")
    args = p.parse_args()
    fleet.up(1, 3, 7100)
    try:
        sessions = create_sessions(args.sessions)
        print(f"{len(sessions)} sessions; {args.rounds} rounds at concurrency {args.concurrency}")
        timings = measure(sessions, args.rounds, args.concurrency)
        ok = True
        print(f"{'call':<18}{'n':>6}{'p50 ms':>10}{'p99 ms':>10}")
        for key in sorted(timings):
            values = timings[key]
            p99 = pct(values, 99)
            print(f"{key:<18}{len(values):>6}{pct(values, 50):>10.1f}{p99:>10.1f}")
            if key.split()[0] in ("upload", "download") and p99 >= GATE_MS:
                ok = False
        print("chaos: uploads while the leader, then a follower, are killed")
        acked, errors, lost = chaos(sessions, args.chaos_seconds, args.concurrency)
        print(f"  {acked} acknowledged uploads, {errors} failed calls, {lost} lost")
        ok = ok and lost == 0 and acked > 0
        print("PASS" if ok else "FAIL")
        sys.exit(0 if ok else 1)
    finally:
        if not args.keep:
            fleet.down()


if __name__ == "__main__":
    main()
