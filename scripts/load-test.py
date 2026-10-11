#!/usr/bin/env python3
"""Drive concurrent llmsim sessions through the public API and report turn latency.

Each session sends sequential user messages; each message carries an llmsim
scenario marker (`[[llmsim:<name>]]`) drawn from a weighted mix, so the
simulated model plays a realistic conversation (tool rounds, long answers,
reasoning, slow first tokens) with no per-model setup. Built-in scenarios are
listed in `crates/drivers/llmsim/scenarios/`; see
`knowledge/operations/load-testing.md` for how runs are read.

    pip install httpx
    # local stack, AUTH_MODE=none
    python3 scripts/load-test.py --api http://localhost:27100/api \
        --model <llmsim model uuid> --sessions 25 --messages 5
    # hosted: an org-scoped key for a dedicated load-testing org
    EVERRUNS_API_KEY=... EVERRUNS_ORG_ID=... python3 scripts/load-test.py \
        --api https://app.everruns.com/api --model <uuid> --ramp 1.1

Prints one JSON object: turn, first-delta and POST percentiles in ms, overall
and per scenario, plus errors and sessions that never went idle.
"""

import argparse
import asyncio
import json
import os
import random
import time

import httpx

DEFAULT_MIX = "chat-short=5,research-3-tools=3,long-answer=1,thinking=1"


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
        for p in (50, 90, 99):
            out[f"{key}_p{p}"] = pct([r[key] for r in rows if r[key] is not None], p)
    return out


class Session:
    """One session: an SSE reader that resumes with since_id, and its turns."""

    def __init__(self, client, api, session_id):
        self.client, self.api, self.id = client, api, session_id
        self.events = asyncio.Queue()
        self.last_id = None
        self.reader = asyncio.create_task(self.read())

    async def read(self):
        # Long idle streams can be cut by a proxy or edge; reconnect like the UI.
        while True:
            params = {"since_id": self.last_id} if self.last_id else {}
            try:
                async with self.client.stream(
                    "GET", f"{self.api}/v1/sessions/{self.id}/sse", params=params, timeout=None
                ) as resp:
                    event = None
                    async for line in resp.aiter_lines():
                        if line.startswith("id:"):
                            self.last_id = line[3:].strip()
                        elif line.startswith("event:"):
                            event = line[6:].strip()
                        elif line == "" and event:
                            await self.events.put((event, time.perf_counter()))
                            event = None
            except (httpx.HTTPError, asyncio.TimeoutError):
                pass
            await asyncio.sleep(0.5)

    async def turn(self, text, timeout):
        while not self.events.empty():
            self.events.get_nowait()
        start = time.perf_counter()
        body = {"message": {"role": "user", "content": [{"type": "text", "text": text}]}}
        resp = await self.client.post(f"{self.api}/v1/sessions/{self.id}/messages", json=body)
        resp.raise_for_status()
        posted, first, failed = time.perf_counter(), None, False
        while True:
            event, at = await asyncio.wait_for(self.events.get(), timeout)
            if first is None and event.endswith(".delta"):
                first = at
            failed |= event == "turn.failed"
            if event == "session.idled":
                if failed:
                    raise RuntimeError("turn.failed")
                ms = lambda t: (t - start) * 1000 if t else None
                return {"post": ms(posted), "first": ms(first), "turn": ms(at)}


async def run(args):
    mix = parse_mix(args.mix)
    names, weights = [n for n, _ in mix], [w for _, w in mix]
    headers = {"User-Agent": "everruns-load-test"}
    if os.environ.get("EVERRUNS_API_KEY"):
        headers["Authorization"] = "Bearer " + os.environ["EVERRUNS_API_KEY"]
    if os.environ.get("EVERRUNS_ORG_ID"):
        headers["X-Org-Id"] = os.environ["EVERRUNS_ORG_ID"]
    limits = httpx.Limits(max_connections=args.sessions * 2 + 20)
    rows, errors, stuck = [], [], []
    ready, go = [0], asyncio.Event()

    async with httpx.AsyncClient(timeout=120, limits=limits, headers=headers) as client:
        harnesses = (await client.get(f"{args.api}/v1/harnesses")).raise_for_status().json()["data"]
        harness = next((h["id"] for h in harnesses if h.get("name") == args.harness), harnesses[0]["id"])
        agent = (
            await client.post(
                f"{args.api}/v1/agents",
                json={
                    "name": f"load-test-{time.time_ns()}",
                    "system_prompt": "Respond concisely.",
                    "default_model_id": args.model,
                },
            )
        ).raise_for_status().json()["id"]

        async def one(i):
            rng = random.Random(f"{args.seed}-{i}")
            session = None
            try:
                await asyncio.sleep(args.ramp * i)
                created = await client.post(
                    f"{args.api}/v1/sessions",
                    json={"harness_id": harness, "agent_id": agent, "model_id": args.model, "title": f"load test {i}"},
                )
                session = Session(client, args.api, created.raise_for_status().json()["id"])
                await asyncio.sleep(0.3)
            except Exception as e:  # noqa: BLE001 - report every failure kind
                errors.append(f"create: {e!r}"[:200])
            ready[0] += 1
            if ready[0] >= args.sessions:
                go.set()
            await go.wait()
            if session is None:
                return
            for m in range(args.messages):
                name = rng.choices(names, weights)[0]
                speed = f" speed={args.speed}" if args.speed != 1 else ""
                text = f"Load test message {m}. [[llmsim:{name} seed={i}{speed}]]"
                try:
                    rows.append({"scenario": name, **await session.turn(text, args.turn_timeout)})
                except asyncio.TimeoutError:
                    stuck.append({"session": session.id, "message": m, "scenario": name})
                    break
                except Exception as e:  # noqa: BLE001
                    errors.append(f"{name}: {e!r}"[:200])
                await asyncio.sleep(args.think)
            session.reader.cancel()

        tasks = [asyncio.create_task(one(i)) for i in range(args.sessions)]
        await go.wait()
        started = time.perf_counter()
        await asyncio.gather(*tasks)
        elapsed = time.perf_counter() - started

    print(
        json.dumps(
            {
                "sessions": args.sessions,
                "messages": args.messages,
                "mix": args.mix,
                "turns_per_s": round(len(rows) / elapsed, 2),
                "errors": len(errors),
                "error_sample": errors[:3],
                "stuck": stuck[:5],
                **summary(rows),
                "by_scenario": {n: summary([r for r in rows if r["scenario"] == n]) for n in names},
            },
            indent=2,
        )
    )


def main():
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
    asyncio.run(run(p.parse_args()))


if __name__ == "__main__":
    main()
