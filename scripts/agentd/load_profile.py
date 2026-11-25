#!/usr/bin/env python3
"""The 75k goal's traffic shapes against a local fleet: the ramp (sessions
created a second, verifier point B3) and the busy peak (turns a second at
three times the pace, B2).

  load_profile.py ramp --rate 41.7 --duration 300
  load_profile.py peak --sessions 6250 --think 10 --minutes 10

Both run 3 shards of 3 nodes with 2 executors and a simulated model, and
print one JSON result last.

- ramp: sessions arrive open loop (Poisson) at `--rate` for `--duration`
  seconds, each running one turn; refused creates are retried. Reports
  sessions created a second, create latency and errors.
- peak: `--sessions` sessions each send a message every `--think` seconds
  (mean, exponential) for `--minutes`, so the offered rate is sessions /
  think turns a second. Reports turns served a second and the share of
  requests shed (429, retried).

Decision: the verifier counts these shared points only from a check aimed
at full-size traffic (points.toml, `scales`), so the verifier passes the
goal's rates, not rates scaled to this machine. A machine that cannot serve
them shows it in the numbers rather than in a smaller target.
"""
import argparse, json, os, subprocess, sys, time

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import fleet  # noqa: E402

DRIVE = os.environ.get("AGENTD_DRIVE_BIN", os.path.join(
    ROOT, os.environ.get("AGENTD_BIN_DIR", os.path.join("target", "release")), "agentd-drive"))
SHARDS = 3

p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
p.add_argument("profile", choices=["ramp", "peak"])
p.add_argument("--rate", type=float, default=41.7, help="ramp: sessions created a second")
p.add_argument("--duration", type=float, default=300, help="ramp: seconds of arrivals")
p.add_argument("--sessions", type=int, default=6250, help="peak: sessions sending turns")
p.add_argument("--think", type=float, default=10.0, help="peak: mean seconds between a session's turns")
p.add_argument("--minutes", type=float, default=10.0, help="peak: how long the peak lasts")
p.add_argument("--agent", default="worker")
p.add_argument("--work-ms", default="50")
a = p.parse_args()

subprocess.run([sys.executable, os.path.join(ROOT, "scripts/agentd/fleet.py"), "down"])
time.sleep(0.5)
subprocess.run(["rm", "-rf", fleet.STATE])
os.environ["AGENTD_WORK_MS"] = a.work_ms
fleet.up(SHARDS, 3, 7100, executors=2)
gws = ",".join(f"http://{fleet.addr(c, n)}" for c in range(SHARDS) for n in (1, 2, 3))

if a.profile == "ramp":
    args = ["--rate", str(a.rate), "--duration", str(a.duration), "--sessions", "0", "--turns", "1"]
else:
    turns = max(1, round(a.minutes * 60 / a.think))
    args = ["--sessions", str(a.sessions), "--concurrency", str(a.sessions),
            "--turns", str(turns), "--think", str(a.think)]
started = time.time()
out = subprocess.run([DRIVE, "load", "--gateways", gws, "--agent", a.agent, "--persistent", *args],
                     capture_output=True, text=True)
subprocess.run([sys.executable, os.path.join(ROOT, "scripts/agentd/fleet.py"), "down"])
try:
    r = json.loads(out.stdout)
except ValueError:
    print(json.dumps({"profile": a.profile, "ok": False, "errors": 1, "err": out.stderr[-2000:]}))
    sys.exit(1)

res = {"profile": a.profile, "shards": SHARDS, "errors": r.get("errors"),
       "first_errors": r.get("first_errors"), "wall_s": round(time.time() - started)}
if a.profile == "ramp":
    res.update({"offered_per_sec": a.rate, "sessions": r.get("sessions"),
                "created_per_sec": round(r.get("created_per_sec") or 0, 2),
                "create_p50_ms": (r.get("create") or {}).get("p50_ms"),
                "create_p99_ms": (r.get("create") or {}).get("p99_ms"),
                "shed": r.get("shed")})
else:
    requests = (r.get("turns_completed") or 0) + (r.get("sessions") or 0) + (r.get("shed") or 0)
    res.update({"offered_turns_per_sec": round(a.sessions / a.think, 1), "sessions": r.get("sessions"),
                "turns": r.get("turns_completed"),
                # Turns finished over the run, creates included in the wall time.
                "turns_per_sec": round(r.get("turns_per_sec") or 0, 1),
                "shed": r.get("shed"), "shed_share": round((r.get("shed") or 0) / max(requests, 1), 5),
                "turn_p50_ms": (r.get("turn") or {}).get("p50_ms"),
                "turn_p99_ms": (r.get("turn") or {}).get("p99_ms")})
res["ok"] = (r.get("errors") or 0) == 0
print(json.dumps(res, indent=1))
sys.exit(0 if res["ok"] else 1)
