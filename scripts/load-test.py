#!/usr/bin/env python3
"""Drive concurrent llmsim sessions through the public API and report turn latency.

Each session sends sequential user messages; each message carries an llmsim
scenario marker (`[[llmsim:<name>]]`) drawn from a weighted mix, so the
simulated model plays a realistic conversation (tool rounds, long answers,
reasoning, slow first tokens) with no per-model setup. Built-in scenarios come
from llmsim's `ScenarioSet::builtin()` (its `examples/scenarios/`); see
`knowledge/operations/load-testing.md` for how runs are read.

    pip install httpx
    # local stack, AUTH_MODE=none
    python3 scripts/load-test.py --api http://localhost:27100/api \
        --model <llmsim model uuid> --sessions 25 --messages 5
    # hosted: an org-scoped key for a dedicated load-testing org
    EVERRUNS_API_KEY=... EVERRUNS_ORG_ID=... python3 scripts/load-test.py \
        --api https://app.everruns.com/api --model <uuid> --ramp 1.1

Long-session and client realism (all optional, all seeded):

    --file-kb 6,60          seeded file size per session, log-normal median,p99 KB
    --duration 3600         soak: each session keeps talking until the deadline
    --idle 20,600           log-normal gap between a session's messages (s)
    --reopen-after 120      a gap this long closes the SSE stream; the session
                            comes back by reloading its whole history (timed)
    --herd-every 300        every N s a --herd-fraction of idle sessions send at
                            once, like users returning after a deploy
    --tabs 3                SSE streams per session, as several open tabs
    --abandon 0.05          chance a session sends and closes before the reply
    --arrival poisson       exponential session arrivals with mean --ramp
    --switch-model <uuid>   switch the session model every --switch-every turns
    --verify 20             check the first N sessions' persisted history: every
                            sent message once, in order (checksum nonce ck=...)
    --capabilities a,b      agent capabilities, e.g. bashkit_shell,subagents
    --save-baseline f.json  write this run's result
    --baseline f.json       fail when p50/p95 turn or first delta regress by more
                            than --tolerance (default 0.2) against a saved run

Prints one JSON object: turn, first-delta and POST percentiles in ms, overall
and per scenario, plus errors and sessions that never went idle.
"""

import argparse
import asyncio
import hashlib
import math
from contextlib import AsyncExitStack
import json
import os
import random
import time

import httpx

DEFAULT_MIX = "chat-short=5,file-research=3,long-answer=1,thinking=1"
# Scenarios that read /load-test/research.md, so each session is seeded first.
SEEDED_SCENARIOS = {"file-research", "luna-agent", "sol-agent", "muse-agent"}

RESEARCH_NOTES = "# Research notes\n\n" + (
    "The runtime stores session events durably, streams replies, and distributes turns across workers.\n"
    * 40
)


# z-score of the 99th percentile of a standard normal.
Z99 = 2.326


def parse_range(text):
    """`median,p99` as two positive floats with p99 >= median."""
    median, _, p99 = text.partition(",")
    median, p99 = float(median), float(p99 or median)
    if median <= 0 or p99 < median:
        raise argparse.ArgumentTypeError("expected median,p99 with 0 < median <= p99")
    return median, p99


def lognormal(rng, median, p99):
    """A log-normal draw with this median and p99, capped at 4x p99."""
    sigma = math.log(p99 / median) / Z99
    return min(median * math.exp(sigma * rng.gauss(0, 1)), 4 * p99)


def research_notes(kb=None):
    """The seeded research file, about `kb` KB when given."""
    if kb is None:
        return RESEARCH_NOTES
    line = "The runtime stores session events durably, streams replies, and distributes turns across workers.\n"
    return "# Research notes\n\n" + line * max(1, round(kb * 1024 / len(line)))


def arrivals(args):
    """Seconds after start at which each session is created."""
    if args.arrival != "poisson" or args.ramp <= 0:
        return [args.ramp * i for i in range(args.sessions)]
    rng, at, out = random.Random(f"{args.seed}-arrival"), 0.0, []
    for _ in range(args.sessions):
        out.append(at)
        at += rng.expovariate(1 / args.ramp)
    return out


def nonce(seed, session, message):
    return hashlib.sha256(f"{seed}-{session}-{message}".encode()).hexdigest()[:12]


async def history(client, api, session_id):
    """Every persisted event of a session, oldest first, as a reopened tab loads it."""
    out, cursor = [], 0
    while True:
        page = (await client.get(
            f"{api}/v1/platform/sessions/{session_id}/events", params={"after_sequence": cursor, "limit": 1000}
        )).raise_for_status().json()
        items = page.get("data", page) if isinstance(page, dict) else page
        out.extend(items)
        seqs = [e["sequence"] for e in items if e.get("sequence") is not None]
        if len(items) < 1000 or not seqs:
            return out
        cursor = max(seqs)


def check_integrity(events, sent):
    """Problems with the persisted user messages: each sent nonce once, in order."""
    seen = []
    for event in sorted(events, key=lambda e: e.get("sequence") or 0):
        if (event.get("type") or event.get("event_type")) != "input.message":
            continue
        blob = json.dumps(event.get("data"))
        seen.extend(n for n in sent if f"ck={n}" in blob)
    problems = []
    for n in sent:
        count = seen.count(n)
        if count != 1:
            problems.append(f"ck={n} persisted {count} times")
    if not problems and seen != sent:
        problems.append("messages persisted out of order")
    return problems


BASELINE_KEYS = ("turn_p50", "turn_p95", "first_p50", "first_p95")
# Differences under this many ms are noise, whatever the ratio.
BASELINE_FLOOR_MS = 50


def compare_baseline(result, baseline, tolerance):
    """Metrics worse than the baseline by more than `tolerance` (a fraction)."""
    regressions = []
    for key in BASELINE_KEYS:
        now, then = result.get(key), baseline.get(key)
        if now is None or then is None:
            continue
        if now > then * (1 + tolerance) and now - then > BASELINE_FLOOR_MS:
            regressions.append({"metric": key, "baseline": then, "now": now})
    return regressions


def load_json(path):
    with open(path) as f:
        return json.load(f)


def save_json(path, value):
    with open(path, "w") as f:
        json.dump(value, f, indent=2)


def parse_mix(text):
    mix = []
    for part in text.split(","):
        name, _, weight = part.partition("=")
        mix.append((name.strip(), float(weight or 1)))
    return mix


def pct(values, p):
    if not values:
        return None
    values = sorted(values)
    return round(values[min(len(values) - 1, int(round(p / 100 * (len(values) - 1))))])


def summary(rows):
    out = {"turns": len(rows)}
    for key in ("turn", "first", "post"):
        values = [r[key] for r in rows if r[key] is not None]
        for p in (50, 90, 95, 99):
            out[f"{key}_p{p}"] = pct(values, p)
        out[f"{key}_avg"] = round(sum(values) / len(values), 2) if values else None
    return out


async def validate_simulated_model(client, api, model_id):
    model = (await client.get(f"{api}/v1/platform/models/{model_id}")).raise_for_status().json()
    provider = (await client.get(f"{api}/v1/platform/providers/{model['provider_id']}")).raise_for_status().json()
    if provider["provider_type"] != "llmsim" or not model["enabled"]:
        raise ValueError("Load testing requires an enabled llmsim model")


class Session:
    """One session: an SSE reader that resumes with since_id, and its turns."""

    def __init__(self, client, api, session_id, keep=True):
        # `keep=False` is an extra tab: it holds a stream and drops what it reads.
        self.client, self.api, self.id, self.keep = client, api, session_id, keep
        self.events = asyncio.Queue()
        self.last_id = None
        self.connected = asyncio.Event()
        self.stream_errors = 0
        self.reader = asyncio.create_task(self.read())

    async def read(self):
        # Long idle streams can be cut by a proxy or edge; reconnect like the UI.
        while True:
            params = {"since_id": self.last_id} if self.last_id else {}
            try:
                async with self.client.stream(
                    "GET", f"{self.api}/v1/platform/sessions/{self.id}/sse", params=params, timeout=None
                ) as resp:
                    resp.raise_for_status()
                    self.connected.set()
                    event = None
                    async for line in resp.aiter_lines():
                        if line.startswith("id:"):
                            self.last_id = line[3:].strip()
                        elif line.startswith("event:"):
                            event = line[6:].strip()
                        elif line == "" and event:
                            if self.keep:
                                await self.events.put((event, time.perf_counter()))
                            event = None
            except (httpx.HTTPError, asyncio.TimeoutError):
                self.stream_errors += 1
            await asyncio.sleep(0.5)

    async def post(self, text):
        body = {"message": {"role": "user", "content": [{"type": "text", "text": text}]}}
        (await self.client.post(f"{self.api}/v1/platform/sessions/{self.id}/messages", json=body)).raise_for_status()

    async def turn(self, text, timeout):
        while not self.events.empty():
            self.events.get_nowait()
        start = time.perf_counter()
        await self.post(text)
        posted, first, failed = time.perf_counter(), None, False
        while True:
            event, at = await asyncio.wait_for(self.events.get(), timeout)
            if first is None and event.endswith(".delta"):
                first = at
            failed |= event in ("turn.failed", "turn.sealed")
            if event == "session.idled":
                if failed:
                    raise RuntimeError("turn failed or sealed")
                ms = lambda t: (t - start) * 1000 if t else None
                return {"post": ms(posted), "first": ms(first), "turn": ms(at)}


DEFAULTS = {
    "messages": 5, "speed": 1, "think": 2, "ramp": 0, "harness": "generic", "seed": "0",
    "turn_timeout": 120, "file_kb": None, "duration": None, "idle": None, "reopen_after": None,
    "herd_every": None, "herd_fraction": 0.5, "tabs": 1, "abandon": 0, "arrival": "even",
    "switch_model": None, "switch_every": 3, "verify": 0, "capabilities": "",
    "baseline": None, "save_baseline": None, "tolerance": 0.2, "sessions": 10,
}


async def run(args):
    for key, value in DEFAULTS.items():
        if not hasattr(args, key):
            setattr(args, key, value)
    mix = parse_mix(args.mix)
    names, weights = [n for n, _ in mix], [w for _, w in mix]
    headers = {"User-Agent": "everruns-load-test"}
    if os.environ.get("EVERRUNS_API_KEY"):
        headers["Authorization"] = "Bearer " + os.environ["EVERRUNS_API_KEY"]
    if os.environ.get("EVERRUNS_ORG_ID"):
        headers["X-Org-Id"] = os.environ["EVERRUNS_ORG_ID"]
    rows, errors, stuck, sessions = [], [], [], []
    reloads, integrity = [], {"checked": 0, "failures": []}
    counts = {"planned": 0, "abandoned": 0, "switches": 0, "herd_turns": 0}
    ready, go = [0], asyncio.Event()
    herd = [asyncio.Event()]
    deadline = [None]

    async with AsyncExitStack() as stack:
        client = await stack.enter_async_context(httpx.AsyncClient(timeout=120, headers=headers))
        # Fail before creating work if a supplied model can call a real provider.
        await validate_simulated_model(client, args.api, args.model)
        if args.switch_model:
            await validate_simulated_model(client, args.api, args.switch_model)
        harnesses = (await client.get(f"{args.api}/v1/platform/harnesses")).raise_for_status().json()["data"]
        harness = next((h["id"] for h in harnesses if h.get("name") == args.harness), harnesses[0]["id"])
        agent_body = {
            "name": f"load-test-{time.time_ns()}",
            "system_prompt": "Respond concisely.",
            "default_model_id": args.model,
        }
        if args.capabilities:
            agent_body["capabilities"] = [{"ref": c.strip()} for c in args.capabilities.split(",") if c.strip()]
        agent = (await client.post(f"{args.api}/v1/platform/agents", json=agent_body)).raise_for_status().json()["id"]

        # httpcore assigns queued requests by scanning the pool. A single pool
        # with thousands of SSE connections stalls the generator's event loop
        # during bursts, inflating client latency and delaying metrics samples.
        shard_size = 100
        clients = [await stack.enter_async_context(httpx.AsyncClient(
            timeout=120, headers=headers,
            limits=httpx.Limits(max_connections=shard_size * (args.tabs + 1) + 20),
        )) for _ in range((args.sessions + shard_size - 1) // shard_size)]
        starts = arrivals(args)

        async def open_streams(client, session_id):
            main = Session(client, args.api, session_id)
            tabs = [Session(client, args.api, session_id, keep=False) for _ in range(args.tabs - 1)]
            sessions.extend([main, *tabs])
            await asyncio.wait_for(main.connected.wait(), args.turn_timeout)
            return main, tabs

        def close(streams):
            for stream in streams:
                stream.reader.cancel()

        async def gap(rng, session, tabs, client):
            """Wait between messages; returns (session, tabs, joined_herd)."""
            seconds = lognormal(rng, *args.idle) if args.idle else args.think
            if deadline[0] is not None:
                seconds = max(0.0, min(seconds, deadline[0] - time.perf_counter()))
            reopen = args.reopen_after is not None and seconds >= args.reopen_after
            if reopen:
                close([session, *tabs])
            joined, end = False, time.perf_counter() + seconds
            while (left := end - time.perf_counter()) > 0:
                bell = herd[0]
                try:
                    await asyncio.wait_for(bell.wait(), left)
                except asyncio.TimeoutError:
                    break
                if rng.random() < args.herd_fraction:
                    joined = True
                    break
                # Not joining this herd: wait for the next bell or the gap end.
                while herd[0] is bell and time.perf_counter() < end:
                    await asyncio.sleep(min(0.05, max(0.0, end - time.perf_counter())))
            if reopen:
                # A returning user's tab loads the whole history, then streams.
                started = time.perf_counter()
                await history(client, args.api, session.id)
                session, tabs = await open_streams(client, session.id)
                reloads.append({"reload": (time.perf_counter() - started) * 1000})
            return session, tabs, joined

        async def one(i):
            client = clients[i // shard_size]
            rng = random.Random(f"{args.seed}-{i}")
            session, tabs, sent = None, [], []
            try:
                await asyncio.sleep(starts[i])
                created = await client.post(
                    f"{args.api}/v1/platform/sessions",
                    json={"harness_id": harness, "agent_id": agent, "model_id": args.model, "title": f"load test {i}"},
                )
                session_id = created.raise_for_status().json()["id"]
                if SEEDED_SCENARIOS.intersection(names) or args.file_kb:
                    # Seed each session's own VFS before timing turns. Explicit
                    # local tools avoid random tools with invalid or external inputs.
                    kb = lognormal(rng, *args.file_kb) if args.file_kb else None
                    (await client.post(
                        f"{args.api}/v1/platform/sessions/{session_id}/fs/load-test/research.md",
                        json={"content": research_notes(kb), "encoding": "text"},
                    )).raise_for_status()
                session, tabs = await open_streams(client, session_id)
            except Exception as e:  # noqa: BLE001 - report every failure kind
                errors.append(f"create: {e!r}"[:200])
            ready[0] += 1
            if ready[0] >= args.sessions:
                go.set()
            await go.wait()
            if session is None:
                return
            if not session.connected.is_set():
                close([session, *tabs])
                return
            m, joined = 0, False
            try:
                while (time.perf_counter() < deadline[0]) if deadline[0] is not None else m < args.messages:
                    if args.switch_model and m and m % args.switch_every == 0:
                        model = args.switch_model if (m // args.switch_every) % 2 else args.model
                        (await client.patch(
                            f"{args.api}/v1/platform/sessions/{session.id}", json={"model_id": model}
                        )).raise_for_status()
                        counts["switches"] += 1
                    name = rng.choices(names, weights)[0]
                    speed = f" speed={args.speed}" if args.speed != 1 else ""
                    check = ""
                    if i < args.verify:
                        sent.append(nonce(args.seed, i, m))
                        check = f" ck={sent[-1]}"
                    text = f"Load test message {m}.{check} [[llmsim:{name} seed={i}{speed}]]"
                    if args.abandon and rng.random() < args.abandon:
                        # Send, then close every tab before the reply arrives.
                        await session.post(text)
                        await asyncio.sleep(rng.uniform(0, 1))
                        counts["abandoned"] += 1
                        break
                    counts["planned"] += 1
                    try:
                        row = await session.turn(text, args.turn_timeout)
                        rows.append({"scenario": name, "herd": joined, **row})
                        counts["herd_turns"] += joined
                    except asyncio.TimeoutError:
                        stuck.append({"session": session.id, "message": m, "scenario": name})
                        break
                    except Exception as e:  # noqa: BLE001
                        errors.append(f"{name}: {e!r}"[:200])
                    m += 1
                    if deadline[0] is not None and time.perf_counter() >= deadline[0]:
                        break
                    session, tabs, joined = await gap(rng, session, tabs, client)
            except Exception as e:  # noqa: BLE001
                errors.append(f"session: {e!r}"[:200])
            close([session, *tabs])
            if sent:
                integrity["checked"] += 1
                try:
                    problems = check_integrity(await history(client, args.api, session.id), sent)
                except Exception as e:  # noqa: BLE001
                    problems = [f"history: {e!r}"[:200]]
                integrity["failures"].extend({"session": session.id, "problem": p} for p in problems)

        async def bells():
            while True:
                await asyncio.sleep(args.herd_every)
                bell, herd[0] = herd[0], asyncio.Event()
                bell.set()

        tasks = [asyncio.create_task(one(i)) for i in range(args.sessions)]
        await go.wait()
        started = time.perf_counter()
        if args.duration:
            deadline[0] = started + args.duration
        ringer = asyncio.create_task(bells()) if args.herd_every else None
        await asyncio.gather(*tasks)
        if ringer:
            ringer.cancel()
        await asyncio.gather(*(session.reader for session in sessions), return_exceptions=True)
        elapsed = time.perf_counter() - started

    result = {
        "sessions": args.sessions,
        "messages": args.messages if not args.duration else None,
        "duration_s": args.duration,
        "mix": args.mix,
        "turns_per_s": round(len(rows) / elapsed, 2) if elapsed else None,
        "errors": len(errors),
        "error_sample": errors[:3],
        "stuck": stuck[:5],
        "stuck_count": len(stuck),
        "stream_errors": sum(session.stream_errors for session in sessions),
        **{k: v for k, v in counts.items() if k != "planned"},
        **summary(rows),
        "by_scenario": {n: summary([r for r in rows if r["scenario"] == n]) for n in names},
    }
    if args.herd_every:
        result["herd"] = summary([r for r in rows if r["herd"]])
    if reloads:
        values = [r["reload"] for r in reloads]
        result["reloads"] = {"count": len(values), **{f"reload_p{p}": pct(values, p) for p in (50, 95, 99)}}
    if args.verify:
        result["integrity"] = {"checked": integrity["checked"], "failures": integrity["failures"][:10],
                               "failure_count": len(integrity["failures"])}
    regressions = []
    if args.baseline:
        regressions = compare_baseline(result, load_json(args.baseline), args.tolerance)
        result["baseline"] = {"file": args.baseline, "tolerance": args.tolerance, "regressions": regressions}
    passed = (bool(rows) or counts["abandoned"] > 0) and len(rows) == counts["planned"] and not errors \
        and not stuck and not integrity["failures"] and not regressions
    result["passed"] = passed
    if args.save_baseline:
        save_json(args.save_baseline, result)
    print(json.dumps(result, indent=2))
    return passed


def positive_int(text):
    value = int(text)
    if value < 1:
        raise argparse.ArgumentTypeError("must be at least 1")
    return value


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    p.add_argument("--api", default="http://localhost:27100/api")
    p.add_argument("--model", required=True, help="id of an enabled llmsim model")
    p.add_argument("--sessions", type=int, default=10)
    p.add_argument("--messages", type=int, default=5, help="user messages per session")
    p.add_argument("--mix", default=DEFAULT_MIX, help="scenario=weight,... (default: %(default)s)")
    p.add_argument("--speed", type=float, default=1, help="scale scenario timing; 0.5 runs twice as fast")
    p.add_argument("--think", type=float, default=2, help="seconds between a session's messages")
    p.add_argument("--ramp", type=float, default=0, help="seconds between session creations")
    p.add_argument("--harness", default="generic")
    p.add_argument("--seed", default="0")
    p.add_argument("--turn-timeout", type=float, default=120)
    p.add_argument("--file-kb", type=parse_range, help="seeded file size, median,p99 KB")
    p.add_argument("--duration", type=float, help="soak: talk until this many seconds pass (ignores --messages)")
    p.add_argument("--idle", type=parse_range, help="gap between messages, median,p99 s (replaces --think)")
    p.add_argument("--reopen-after", type=float, help="gaps this long close the stream and reload history")
    p.add_argument("--herd-every", type=float, help="seconds between herd bells")
    p.add_argument("--herd-fraction", type=float, default=0.5, help="share of idle sessions joining a herd")
    p.add_argument("--tabs", type=positive_int, default=1, help="SSE streams per session")
    p.add_argument("--abandon", type=float, default=0, help="chance per message to send and close")
    p.add_argument("--arrival", choices=("even", "poisson"), default="even")
    p.add_argument("--switch-model", help="second llmsim model id to switch to and back")
    p.add_argument("--switch-every", type=positive_int, default=3, help="turns between model switches")
    p.add_argument("--verify", type=int, default=0, help="check persisted history of the first N sessions")
    p.add_argument("--capabilities", default="", help="agent capability refs, comma separated")
    p.add_argument("--baseline", help="saved result to compare against")
    p.add_argument("--save-baseline", help="write this run's result here")
    p.add_argument("--tolerance", type=float, default=0.2, help="allowed regression vs --baseline")
    raise SystemExit(0 if asyncio.run(run(p.parse_args(argv))) else 1)

if __name__ == "__main__":
    import sys
    if "--capacity" in sys.argv:
        from capacity.run import main as capacity_main
        capacity_main([arg for arg in sys.argv[1:] if arg != "--capacity"])
    else:
        main()
