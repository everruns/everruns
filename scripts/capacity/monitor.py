"""Independent-process resource sampling; no command lines, SQL text or secrets.

PID counters and aggregate database pressure help distinguish platform limits
from generator contention. Gaps and sample errors are part of the result.
"""

import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import time
from urllib.parse import urlsplit, unquote


def process_sample(pid):
    path = Path(f"/proc/{pid}/stat")
    if not path.exists() and __import__("platform").system() == "Darwin":
        result = subprocess.run(
            ["ps", "-p", str(pid), "-o", "time=,rss="],
            capture_output=True,
            text=True,
            check=True,
        )
        cpu, rss = result.stdout.split()
        parts = [float(value) for value in cpu.split(":")]
        seconds = sum(value * (60**i) for i, value in enumerate(reversed(parts)))
        return {
            "pid": pid,
            "cpu_seconds": seconds,
            "rss_bytes": int(rss) * 1024,
            "open_fds": None,
        }
    stat = path.read_text().rsplit(")", 1)[1].split()
    page = os.sysconf("SC_PAGE_SIZE")
    hz = os.sysconf("SC_CLK_TCK")
    return {
        "pid": pid,
        "cpu_seconds": (int(stat[11]) + int(stat[12])) / hz,
        "rss_bytes": int(stat[21]) * page,
        "open_fds": len(list(Path(f"/proc/{pid}/fd").iterdir())),
    }


def database_sample():
    url = os.environ.get("CAPACITY_DATABASE_URL")
    if not url:
        return None
    parsed = urlsplit(url)
    env = {
        **os.environ,
        "PGHOST": parsed.hostname,
        "PGPORT": str(parsed.port or 5432),
        "PGUSER": unquote(parsed.username or ""),
        "PGPASSWORD": unquote(parsed.password or ""),
        "PGDATABASE": parsed.path.lstrip("/"),
    }
    query = """SELECT json_build_object(
      'connections',(SELECT count(*) FROM pg_stat_activity),
      'active',(SELECT count(*) FROM pg_stat_activity WHERE state='active'),
      'lock_waiters',(SELECT count(*) FROM pg_stat_activity WHERE wait_event_type='Lock'),
      'commits',xact_commit,'rollbacks',xact_rollback,'deadlocks',deadlocks,
      'temp_bytes',temp_bytes,'database_bytes',pg_database_size(datname))
      FROM pg_stat_database WHERE datname=current_database()"""
    result = subprocess.run(
        ["psql", "-XAt", "-c", query],
        env=env,
        capture_output=True,
        text=True,
        timeout=5,
    )
    if result.returncode:
        raise RuntimeError("database sampler failed")
    return json.loads(result.stdout)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument(
        "--pids", required=True, help="JSON map from non-secret role label to PID"
    )
    p.add_argument("--output", required=True)
    p.add_argument("--stop-file", required=True)
    p.add_argument("--interval", type=float, default=2)
    a = p.parse_args()
    start = last = time.monotonic()
    with open(a.output, "w", buffering=1) as output:
        while not Path(a.stop_file).exists():
            now = time.monotonic()
            row = {
                "epoch": time.time(),
                "elapsed": now - start,
                "sample_gap": now - last,
                "processes": {},
            }
            try:
                pids = json.loads(Path(a.pids).read_text())
                if "postgres" in pids:
                    listing = subprocess.run(
                        ["ps", "-axo", "pid=,ppid="],
                        capture_output=True,
                        text=True,
                        check=True,
                    )
                    children = {}
                    for line in listing.stdout.splitlines():
                        child, parent = map(int, line.split())
                        children.setdefault(parent, []).append(child)
                    pending = list(children.get(pids["postgres"], []))
                    while pending:
                        child = pending.pop()
                        pids[f"postgres/{child}"] = child
                        pending.extend(children.get(child, []))
                for name, pid in pids.items():
                    try:
                        value = process_sample(int(pid))
                        smaps = Path(f"/proc/{pid}/smaps_rollup")
                        if smaps.exists():
                            for line in smaps.read_text().splitlines():
                                if line.startswith("Pss:"):
                                    value["pss_bytes"] = int(line.split()[1]) * 1024
                        row["processes"][name] = value
                    except (FileNotFoundError, ProcessLookupError):
                        row["processes"][name] = {"missing": True}
                row["disk_free_bytes"] = shutil.disk_usage(a.output).free
                for file in ["cpu.stat", "memory.current", "memory.max", "cpu.max"]:
                    path = Path("/sys/fs/cgroup") / file
                    if path.exists():
                        row[file] = path.read_text().strip()
                row["database"] = database_sample()
            except Exception as error:
                row["error"] = type(error).__name__
            output.write(json.dumps(row) + "\n")
            last = now
            time.sleep(max(0, a.interval - (time.monotonic() - now)))


if __name__ == "__main__":
    main()
