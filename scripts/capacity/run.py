"""Measured steady load and bursts on the three authenticated chat paths.

A unique input marker and retained idempotency key join client observations to
persisted events. HTTP success alone is not delivery proof. Generator delays,
setup failures, 429s and audit failures remain failures, never discarded samples.
"""

from __future__ import annotations

import argparse
import asyncio
from contextlib import AsyncExitStack
from collections import Counter
from datetime import datetime
import json
from pathlib import Path
import random
import math
import ssl
import time
import uuid

import httpx

from .protocol import (
    load_target,
    preflight,
    authenticated_request,
    verified_identity,
    trust_environment,
)
from .stream import Stream


def percentiles(values):
    if not values:
        return {"count": 0}
    values = sorted(values)
    return {
        "count": len(values),
        "mean": sum(values) / len(values),
        **{
            f"p{p}": values[min(len(values) - 1, int((len(values) - 1) * p / 100))]
            for p in [50, 95, 99]
        },
    }


def queue_metrics(rows, duration):
    """Client-observed pickup trend; completion/schedule gates bound unfinished work."""
    windows = [[], []]
    for row in rows:
        if row.get("pickup_ms") is None:
            continue
        if row["start_seconds"] < duration / 3:
            windows[0].append(row["pickup_ms"])
        if row["start_seconds"] >= duration * 2 / 3:
            windows[1].append(row["pickup_ms"])
    early, late = [percentiles(values) for values in windows]
    measured = min(early["count"], late["count"]) >= 10
    return {
        "early_pickup_ms": early,
        "late_pickup_ms": late,
        "trend_measured": measured,
        "stable": None if not measured else late["p95"] <= max(1000, early["p95"] * 2),
    }


def verdict(report, ack_p95):
    admission = report["admissions"]
    attempted = report["attempted"]
    checks = {
        "full_steady_duration_observed": report.get("phase") != "steady"
        or report.get("duration_seconds", 0)
        >= report.get("requested_duration_seconds", 0),
        "all_users_connected": report["connected_users"] == report["users"],
        "all_scheduled_users_ran": report.get("started_users", report["users"])
        == report.get("scheduled_users", report["users"]),
        "all_scheduled_messages_attempted": report["attempted"]
        == report.get("planned_attempts", report["attempted"]),
        "admission_success_99_9": attempted > 0 and admission / attempted >= 0.999,
        "ack_p95_ms": admission > 0
        and report["ack_ms"].get("p95", float("inf")) < ack_p95,
        "all_admitted_turns_complete": report["completed"] == admission,
        "no_lost_or_duplicated_inputs": report["audit_errors"] == 0
        and report["audited_inputs"] == admission,
        "no_missing_streamed_inputs": report["stream_missing_inputs"] == 0,
        "workload_verified": report["workload_audit_errors"] == 0
        and report["workload_verified_turns"] == report["completed"],
        "requested_reconnect_executed": report.get("workload", {}).get(
            "reconnect_at_seconds", -1
        )
        < 0
        or report.get("reconnected_users", 0) == report["users"],
        "no_background_request_errors": report.get("fault_mode", False)
        or not report["poll_errors"],
        "no_unexpected_stream_errors": report.get("fault_mode", False)
        or not report.get("stream_errors", 0),
        "no_setup_stream_errors": not report.get("setup_stream_errors", 0),
        "no_setup_audit_or_schedule_errors": not any(
            not key.startswith("turn:") or key == "turn:ValueError"
            for key in report["errors"]
        ),
        "no_failed_tools": not report.get("failed_tools", 0),
        "no_terminal_stream_errors": not report.get("terminal_stream_errors", 0),
        "no_sustained_queue_growth": report.get("fault_mode", False)
        or report.get("phase") != "steady"
        or report.get("queue", {}).get("stable") is not False,
        "generator_on_schedule": report.get("arrival_lag_ms", {}).get("p95", 0) < 100,
    }
    return {"passed": all(checks.values()), "checks": checks}


class Results:
    def __init__(self, output):
        self.rows = []
        self.errors = Counter()
        self.http_status = Counter()
        self.poll_errors = Counter()
        self.poll_ms = []
        self.start_lag_ms = []
        self.poll_requests = 0
        self.output = output
        self.handle = open(output / "turns.jsonl", "a", buffering=1)

    def record(self, row):
        self.rows.append(row)
        self.handle.write(json.dumps(row) + "\n")

    def error(self, kind, error):
        # Never interpolate HTTPError/response bodies: they can contain credentials.
        code = (
            str(error.response.status_code)
            if isinstance(error, httpx.HTTPStatusError)
            else type(error).__name__
        )
        self.errors[f"{kind}:{code}"] += 1


async def post_message(
    client, actor, body, key, results, lose_ack=False, retry_transient=False
):
    url, payload = actor.send(body, key)
    deadline = time.monotonic() + 15
    delay = 0.25
    while True:
        try:
            response = await authenticated_request(
                client,
                actor,
                "POST",
                url,
                json=payload,
                headers={"Idempotency-Key": key},
            )
            results.http_status[str(response.status_code)] += 1
            response.raise_for_status()
            break
        except (httpx.TransportError, httpx.HTTPStatusError) as error:
            status = (
                error.response.status_code
                if isinstance(error, httpx.HTTPStatusError)
                else None
            )
            if (
                not retry_transient
                or time.monotonic() + delay > deadline
                or (status is not None and status != 409 and status < 500)
            ):
                raise
            await asyncio.sleep(delay)
            delay = min(delay * 2, 2)
    original = (
        response.json().get("output", {})
        if actor.path == "console"
        else response.json()
    )
    if lose_ack:
        # Admit the original, discard its response, and send exactly the same intent.
        # Independent reconnect tests exercise a real closed stream; this injection
        # deliberately qualifies ambiguous acknowledgment, not a packet-loss model.
        for _ in range(5):
            replay = await authenticated_request(
                client,
                actor,
                "POST",
                url,
                json=payload,
                headers={"Idempotency-Key": key},
            )
            results.http_status[str(replay.status_code)] += 1
            if replay.status_code != 409:
                replay.raise_for_status()
                value = (
                    replay.json().get("output", {})
                    if actor.path == "console"
                    else replay.json()
                )
                if value.get("id") != original.get("id"):
                    raise ValueError("idempotency returned a different message")
                return original
            await asyncio.sleep(0.25)
        raise ValueError("idempotency remained in progress")
    return original


async def one_turn(
    client, actor, stream, index, sequence, args, results, scheduled=None
):
    # Drain old terminal/control frames only after the previous turn settled.
    while not stream.queue.empty():
        stream.queue.get_nowait()
    key = str(uuid.uuid4())
    marker = f"capacity:{args.run_id}:{index}:{sequence}"
    rng = random.Random(f"{args.seed}:{index}:{sequence}")
    shapes = [
        ("chat-short", 5),
        ("file-research", 3),
        ("long-answer", 1),
        ("thinking", 1),
    ]
    shape = (
        rng.choices([x[0] for x in shapes], [x[1] for x in shapes])[0]
        if args.mix == "realistic"
        else args.mix
    )
    simulator_shape = (
        "console-file-research"
        if actor.path == "console" and shape == "file-research"
        else shape
    )
    text = f"{marker} [[llmsim:{simulator_shape} seed={index}]]"
    row = {
        "actor": index,
        "path": actor.path,
        "marker": marker,
        "scenario": shape,
        "simulator_scenario": simulator_shape,
        "session_id": actor.session,
        "request_id": key,
        "timeout_seconds": args.turn_timeout,
        "admitted": False,
        "completed": False,
        "phase": args.phase,
        "start_seconds": time.perf_counter() - args.started,
        "arrival_lag_ms": max(0, (time.perf_counter() - scheduled) * 1000)
        if scheduled is not None
        else 0,
    }
    start = time.perf_counter()
    try:
        async with asyncio.timeout(args.turn_timeout):
            message = await post_message(
                client,
                actor,
                {
                    "message": {
                        "role": "user",
                        "content": [{"type": "text", "text": text}],
                    }
                },
                key,
                results,
                args.lost_ack_every > 0 and sequence % args.lost_ack_every == 0,
                args.fault_mode,
            )
        row.update(
            admitted=True,
            message_id=message["id"],
            ack_ms=(time.perf_counter() - start) * 1000,
        )
        first = None
        input_ts = started_ts = turn_id = None
        async with asyncio.timeout(
            max(0.001, args.turn_timeout - (time.perf_counter() - start))
        ):
            while True:
                kind, event, at = await stream.queue.get()
                data = event.get("data", {})
                if kind == "input.message":
                    if data.get("message", {}).get("id") != message["id"]:
                        continue
                    input_ts = event.get("ts")
                    row["input_received_ms"] = (at - start) * 1000
                if kind == "turn.started":
                    if data.get("input_message_id") != message["id"] and not (
                        actor.path == "work-chat"
                        and input_ts
                        and "input_message_id" not in data
                    ):
                        continue
                    turn_id = data["turn_id"]
                    started_ts = event.get("ts")
                    row["turn_started_ms"] = (at - start) * 1000
                if kind not in {"input.message", "turn.started"} and (
                    turn_id is None
                    or (
                        data.get("turn_id") != turn_id
                        and not (
                            actor.path == "work-chat"
                            and kind.endswith(".delta")
                            and "turn_id" not in data
                        )
                    )
                ):
                    continue
                if input_ts and started_ts:
                    row["pickup_ms"] = (
                        datetime.fromisoformat(started_ts.replace("Z", "+00:00"))
                        - datetime.fromisoformat(input_ts.replace("Z", "+00:00"))
                    ).total_seconds() * 1000
                if first is None and kind.endswith(".delta"):
                    first = (at - start) * 1000
                if kind in {"turn.failed", "turn.cancelled"}:
                    raise ValueError("turn did not complete")
                if kind == "turn.completed":
                    row.update(
                        completed=True,
                        first_delta_ms=first,
                        turn_ms=(at - start) * 1000,
                    )
                    return True
    except Exception as error:
        results.error("turn", error)
        row["failure_kind"] = (
            f"http_{error.response.status_code}"
            if isinstance(error, httpx.HTTPStatusError)
            else type(error).__name__
        )
        return False
    finally:
        results.record(row)


async def background(client, actor, interval, paths, results, offset, changes=None):
    await asyncio.sleep(offset)
    while True:
        if changes is not None:
            await changes.get()
            await asyncio.sleep(0.25)  # Console coalesces lifecycle invalidations.
            while not changes.empty():
                changes.get_nowait()
        started = time.perf_counter()
        for path in paths:
            results.poll_requests += 1
            try:
                response = await authenticated_request(
                    client, actor, "GET", actor.url(path)
                )
                results.http_status[str(response.status_code)] += 1
                response.raise_for_status()
            except Exception as error:
                code = (
                    error.response.status_code
                    if isinstance(error, httpx.HTTPStatusError)
                    else type(error).__name__
                )
                results.poll_errors[str(code)] += 1
        results.poll_ms.append((time.perf_counter() - started) * 1000)
        if changes is None:
            await asyncio.sleep(max(0, interval - (time.perf_counter() - started)))


async def event_pages(client, target, actor, types, limit):
    before = None
    for _ in range(limit):
        params = {"limit": 1000}
        if before is not None:
            params["before_sequence"] = before
        response = (
            await authenticated_request(
                client,
                actor,
                "GET",
                actor.url(f"sessions/{actor.session}/events"),
                params=params,
            )
            if actor.path == "console"
            else await client.get(
                f"{target.control}/v1/sessions/{actor.session}/events",
                params=params,
                headers=target.headers_for(actor),
            )
        )
        response.raise_for_status()
        data = response.json()
        events = data.get("data", []) if isinstance(data, dict) else data
        if not events:
            break
        yield events
        if len(events) < 1000:
            break
        next_before = min(e["sequence"] for e in events)
        if before is not None and next_before >= before:
            raise ValueError("audit cursor did not advance")
        before = next_before
    else:
        raise ValueError("audit pagination limit reached")


async def audit(client, target, actor, expected, stream, limit):
    found, streamed = Counter(), Counter()
    mismatched = 0
    for message_id, text in stream.inputs.items():
        marker = text.split(" ", 1)[0]
        if marker in expected:
            streamed[marker] += 1
            if isinstance(expected[marker], str) and message_id != expected[marker]:
                mismatched += 1
    async for events in event_pages(client, target, actor, "input.message", limit):
        for event in events:
            if event.get("type") != "input.message":
                continue
            text = "\n".join(
                p.get("text", "")
                for p in event.get("data", {}).get("message", {}).get("content", [])
                if p.get("type") == "text"
            )
            marker = text.split(" ", 1)[0]
            if marker in expected:
                found[marker] += 1
                if (
                    isinstance(expected[marker], str)
                    and event.get("data", {}).get("message", {}).get("id")
                    != expected[marker]
                ):
                    mismatched += 1
    errors = (
        mismatched
        + sum(count != 1 for count in found.values())
        + sum(admitted and marker not in found for marker, admitted in expected.items())
    )
    missing = sum(streamed[marker] != 1 for marker in found)
    return errors, missing, sum(found.values())


def workload_valid(shape, terminal, output):
    # Generous pacing floors plus durable tool/output proof reject llmsim's
    # tiny fallback answer even when queueing makes it appear slow.
    words, tools, calls, milliseconds = {
        "chat-short": (40, 0, 1, 1000),
        "file-research": (600, 4, 4, 10000),
        "long-answer": (1500, 0, 1, 25000),
        "thinking": (300, 0, 1, 10000),
    }[shape]
    text = "\n".join(
        p.get("text", "") for p in output.get("content", []) if p.get("type") == "text"
    )
    observed = {
        "output_words": len(text.split()),
        "tool_calls": terminal.get("tool_call_count") or 0,
        "llm_calls": terminal.get("llm_call_count") or 0,
        "duration_ms": terminal.get("duration_ms") or 0,
    }
    valid = (
        observed["output_words"] >= words
        and observed["tool_calls"] >= tools
        and observed["llm_calls"] >= calls
        and observed["duration_ms"] >= milliseconds
    )
    return valid, observed


def failed_observation(row, input_ts, completion_ts):
    """Use one server clock; within-budget completion does not prove SSE loss.

    Admission and network time consume the client's deadline too. Late durable
    completion remains a failure, never a recovered successful observation.
    """
    if not row.get("admitted"):
        return "acknowledgment_failed", None
    if not input_ts or not completion_ts:
        return "no_durable_completion", None
    elapsed = (
        datetime.fromisoformat(completion_ts.replace("Z", "+00:00"))
        - datetime.fromisoformat(input_ts.replace("Z", "+00:00"))
    ).total_seconds() * 1000
    if elapsed < 0:
        return "inconsistent_server_clock", elapsed
    return (
        "backend_deadline_exceeded"
        if elapsed >= row["timeout_seconds"] * 1000
        else "completed_within_backend_budget",
        elapsed,
    )


async def audit_workload(client, target, actor, rows, limit):
    starts, terminals, outputs = {}, {}, {}
    input_times, completion_times = {}, {}
    async for events in event_pages(
        client,
        target,
        actor,
        "turn.started,turn.completed,output.message.completed",
        limit,
    ):
        for event in events:
            data = event.get("data", {})
            if event["type"] == "input.message":
                input_times[data.get("message", {}).get("id")] = event.get("ts")
            elif event["type"] == "turn.started":
                starts.setdefault(data.get("input_message_id"), []).append(
                    data.get("turn_id")
                )
            elif event["type"] == "turn.completed":
                terminals.setdefault(data.get("turn_id"), []).append(data)
                completion_times[data.get("turn_id")] = event.get("ts")
            elif event["type"] == "output.message.completed":
                message = data.get("message", {})
                outputs.setdefault(message.get("id"), []).append(message)
    proof = []
    for row in rows:
        turns = starts.get(row.get("message_id"), [])
        terminal = terminals.get(turns[0], []) if len(turns) == 1 else []
        if not row["completed"]:
            classification, elapsed = failed_observation(
                row,
                input_times.get(row.get("message_id")),
                completion_times.get(turns[0]) if len(turns) == 1 else None,
            )
            proof.append(
                {
                    "marker": row["marker"],
                    "scenario": row["scenario"],
                    "passed": False,
                    "failure_classification": classification,
                    "backend_elapsed_ms": elapsed,
                }
            )
            continue
        messages = (
            outputs.get(terminal[0].get("final_message_id"), [])
            if len(terminal) == 1
            else []
        )
        valid, observed = workload_valid(
            row["scenario"],
            terminal[0] if len(terminal) == 1 else {},
            messages[0] if len(messages) == 1 else {},
        )
        proof.append(
            {
                "marker": row["marker"],
                "scenario": row["scenario"],
                "passed": valid,
                **observed,
            }
        )
    return proof


async def run(args):
    target = load_target(args.manifest)
    all_actors = [a for a in target.actors if a.path == args.path][: args.users]
    if len(all_actors) != args.users:
        raise ValueError(
            "manifest does not contain enough distinct authenticated actors"
        )
    selected = [
        (i, a) for i, a in enumerate(all_actors) if i % args.shards == args.shard_index
    ]
    if not selected:
        raise ValueError("empty generator shard")
    args.output = Path(args.output)
    args.output.mkdir(parents=True, exist_ok=False)
    results = Results(args.output)
    verify = ssl.create_default_context(cafile=args.ca_bundle)
    streams, sessions, pollers = {}, {}, []
    identities = set()
    transport_versions = Counter()
    ready, go = asyncio.Event(), asyncio.Event()
    ready_count = 0
    started = 0.0
    async with AsyncExitStack() as stack:
        stack.enter_context(results.handle)
        workload_output = stack.enter_context(
            open(args.output / "workload.jsonl", "a", buffering=1)
        )
        control = await stack.enter_async_context(
            httpx.AsyncClient(
                verify=verify, timeout=30, trust_env=trust_environment(target.control)
            )
        )
        await preflight(control, target, all_actors)
        # One transport per actor models independent browsers/SDK clients, while
        # that actor reuses HTTP/2 for its SSE, commands and background reads.
        # Share only CA configuration, not user connections or cookie jars.
        clients = [
            await stack.enter_async_context(
                httpx.AsyncClient(
                    verify=verify,
                    http2=True,
                    timeout=httpx.Timeout(30, pool=30),
                    trust_env=any(trust_environment(a.base) for _, a in selected),
                    limits=httpx.Limits(max_connections=4, max_keepalive_connections=4),
                )
            )
            for _ in selected
        ]
        setup_limit = asyncio.Semaphore(args.setup_concurrency)

        async def actor_job(position, index, actor):
            nonlocal ready_count
            client = clients[position]
            stream = None
            try:
                async with setup_limit:
                    identity = await verified_identity(client, actor)
                    if identity in identities:
                        raise ValueError(
                            "two actors use the same authenticated identity"
                        )
                    identities.add(identity)
                    response = await authenticated_request(
                        client,
                        actor,
                        "POST",
                        actor.url("sessions"),
                        json={
                            **actor.create,
                            "title": f"capacity {args.run_id} {index}",
                        },
                        headers={"Idempotency-Key": str(uuid.uuid4())},
                    )
                    response.raise_for_status()
                    transport_versions[response.http_version] += 1
                    actor.session = response.json()["id"]
                    sessions[index] = actor
                    # Use a dedicated document; never arbitrary external tools.
                    document = {
                        "content": "# Capacity fixture\n"
                        + ("Durable events and simulated tools.\n" * 40),
                        "encoding": "text",
                    }
                    seed = (
                        await authenticated_request(
                            client,
                            actor,
                            "POST",
                            actor.url(
                                f"sessions/{actor.session}/fs/load-test/research.md"
                            ),
                            json=document,
                        )
                        if actor.path == "console"
                        else await control.post(
                            f"{target.control}/v1/sessions/{actor.session}/fs/load-test/research.md",
                            headers=target.headers_for(actor),
                            json=document,
                        )
                    )
                    seed.raise_for_status()
                    stream = Stream(client, actor)
                    streams[index] = stream
                    stream.start()
                    await asyncio.wait_for(stream.ready.wait(), args.turn_timeout)
                    if stream.terminal_error:
                        raise ValueError("SSE authentication failed")
                    if args.background:
                        if actor.path == "console":
                            pollers.append(
                                asyncio.create_task(
                                    background(
                                        client,
                                        actor,
                                        0,
                                        actor.polls()[0][1],
                                        results,
                                        0,
                                        changes=stream.changes,
                                    )
                                )
                            )
                        for interval, paths in actor.polls():
                            pollers.append(
                                asyncio.create_task(
                                    background(
                                        client,
                                        actor,
                                        interval,
                                        paths,
                                        results,
                                        (index % 100) / 100 * interval,
                                    )
                                )
                            )
            except Exception as error:
                results.error("setup", error)
            finally:
                ready_count += 1
                if ready_count == len(selected):
                    ready.set()
            await go.wait()
            if not stream or not stream.connected:
                return
            due = started + (
                0 if args.phase == "burst" else index / args.users * args.interval
            )
            sequence = 0
            while (
                args.phase == "burst"
                and sequence == 0
                or args.phase == "steady"
                and due < started + args.duration
            ):
                await asyncio.sleep(max(0, due - time.perf_counter()))
                results.start_lag_ms.append(max(0, (time.perf_counter() - due) * 1000))
                completed = await one_turn(
                    client, actor, stream, index, sequence, args, results, scheduled=due
                )
                # Work Chat deliberately hides input-to-turn provenance. One
                # outstanding turn, its input echo and terminal turn ID suffice;
                # stop after failure so an old completion cannot settle a new send.
                if not completed:
                    break
                sequence += 1
                if args.phase == "burst":
                    break
                due += args.interval
                # Avoid an invisible catch-up burst when a session is backlogged.
                if due < started + args.duration and due < time.perf_counter():
                    results.errors["arrival_schedule_missed"] += 1
                    due = time.perf_counter() + args.interval

        tasks = [
            asyncio.create_task(actor_job(pos, index, actor))
            for pos, (index, actor) in enumerate(selected)
        ]
        await ready.wait()
        if args.ready_file:
            Path(args.ready_file).write_text(
                json.dumps({"shard_index": args.shard_index, "ready": ready_count})
            )
        if args.start_file:
            deadline = time.monotonic() + args.setup_timeout
            while not Path(args.start_file).exists():
                if time.monotonic() > deadline:
                    raise ValueError("coordinated start timed out")
                await asyncio.sleep(0.25)
            args.start_at = json.loads(Path(args.start_file).read_text())["epoch"]
        if args.start_at is not None:
            wait = args.start_at - time.time()
            if wait < 0:
                for task in tasks:
                    task.cancel()
                await asyncio.gather(*tasks, return_exceptions=True)
                await asyncio.gather(*(s.stop() for s in streams.values()))
                raise ValueError("global start was missed during setup")
            await asyncio.sleep(wait)
        start_wall = time.time()
        started = time.perf_counter()
        args.started = started
        go.set()

        async def faults():
            if args.reconnect_at >= 0:
                await asyncio.sleep(args.reconnect_at)
                await asyncio.gather(
                    *(s.reconnect(args.disconnect_seconds) for s in streams.values())
                )

        fault_task = asyncio.create_task(faults())
        await asyncio.gather(*tasks)
        if args.phase == "steady" and not results.errors:
            # Keep streams and product polls alive for the full requested soak,
            # including when the last scheduled turn completes before its end.
            await asyncio.sleep(max(0, started + args.duration - time.perf_counter()))
        elapsed = time.perf_counter() - started
        # Another generator may still be serving its last long turn. Journal
        # audits add substantial database traffic and must stay outside every
        # generator's timed phase, not just this one's.
        measurement_end = await wait_for_audit(args, start_wall + elapsed)
        fault_task.cancel()
        for poller in pollers:
            poller.cancel()
        await asyncio.gather(fault_task, *pollers, return_exceptions=True)
        # Give queued input echoes a chance to arrive, bounded independently of turn duration.
        await asyncio.sleep(args.audit_grace)
        audit_errors = missing = audited = workload_errors = workload_verified = 0
        audit_slots = asyncio.Semaphore(10)

        async def inspect(index, actor):
            nonlocal audit_errors, missing, audited, workload_errors, workload_verified
            expected = {
                r["marker"]: r.get("message_id", False)
                for r in results.rows
                if r["actor"] == index
            }
            async with audit_slots:
                try:
                    a, b, c = await audit(
                        streams[index].client if actor.path == "console" else control,
                        target,
                        actor,
                        expected,
                        streams[index],
                        args.audit_pages,
                    )
                    audit_errors += a
                    missing += b
                    audited += c
                    proof = await audit_workload(
                        streams[index].client if actor.path == "console" else control,
                        target,
                        actor,
                        [r for r in results.rows if r["actor"] == index],
                        args.audit_pages,
                    )
                    for item in proof:
                        workload_output.write(json.dumps(item) + "\n")
                        workload_errors += not item["passed"]
                        workload_verified += item["passed"]
                except Exception as error:
                    audit_errors += 1
                    results.error("audit", error)

        await asyncio.gather(
            *(inspect(i, a) for i, a in sessions.items() if i in streams)
        )
        reconnected = sum(
            s.forced > 0 and s.connections > 1 and s.connected for s in streams.values()
        )
        await asyncio.gather(*(s.stop() for s in streams.values()))
        pending_sessions = {r["session_id"] for r in results.rows if not r["completed"]}
        for actor in sessions.values():
            # Cancellation is cleanup, never a replayed user approval or tool action.
            if actor.session not in pending_sessions:
                continue
            try:
                client = next(s.client for s in streams.values() if s.actor is actor)
                response = await authenticated_request(
                    client,
                    actor,
                    "POST",
                    actor.url(f"sessions/{actor.session}/cancel"),
                    json={},
                )
                response.raise_for_status()
            except httpx.HTTPError:
                results.errors["cleanup_failed"] += 1
    results.handle.close()
    report = {
        "run_id": args.run_id,
        "path": args.path,
        "phase": args.phase,
        "users": len(selected),
        "total_users": args.users,
        "shards": args.shards,
        "shard_index": args.shard_index,
        "start_epoch": start_wall,
        "fault_mode": args.fault_mode,
        "requested_duration_seconds": args.duration,
        "workload": {
            "seed": args.seed,
            "mix": args.mix,
            "turn_timeout_seconds": args.turn_timeout,
            "lost_ack_every": args.lost_ack_every,
            "reconnect_at_seconds": args.reconnect_at,
            "disconnect_seconds": args.disconnect_seconds,
        },
        "duration_seconds": elapsed,
        "interval_seconds": args.interval,
        "background_polls": args.background,
        "identity_fingerprints": sorted(identities),
        "auth_refreshes": sum(a.refreshes for _, a in selected),
        "connected_users": len([s for s in streams.values() if s.connections]),
        "started_users": len({r["actor"] for r in results.rows}),
        "scheduled_users": sum(
            args.phase == "burst" or index / args.users * args.interval < args.duration
            for index, _ in selected
        ),
        "planned_attempts": len(selected)
        if args.phase == "burst"
        else sum(
            max(
                0,
                math.ceil(
                    (args.duration - index / args.users * args.interval) / args.interval
                ),
            )
            for index, _ in selected
        ),
        "attempted": len(results.rows),
        "admissions": sum(r["admitted"] for r in results.rows),
        "completed": sum(r["completed"] for r in results.rows),
        "audit_errors": audit_errors,
        "audited_inputs": audited,
        "workload_audit_errors": workload_errors,
        "workload_verified_turns": workload_verified,
        "reconnected_users": reconnected,
        "stream_missing_inputs": missing,
        "ack_ms": percentiles([r["ack_ms"] for r in results.rows if r["admitted"]]),
        "pickup_ms": percentiles(
            [r["pickup_ms"] for r in results.rows if "pickup_ms" in r]
        ),
        "turn_ms": percentiles([r["turn_ms"] for r in results.rows if r["completed"]]),
        "first_delta_ms": percentiles(
            [
                r["first_delta_ms"]
                for r in results.rows
                if r.get("first_delta_ms") is not None
            ]
        ),
        "arrival_lag_ms": percentiles(results.start_lag_ms),
        "poll_ms": percentiles(results.poll_ms),
        "poll_errors": dict(results.poll_errors),
        "poll_requests": results.poll_requests,
        "errors": dict(results.errors),
        "http_status": dict(results.http_status),
        "http_versions": dict(transport_versions),
        "stream_connections": sum(s.connections for s in streams.values()),
        **{
            key: sum(
                s.error_stats(start_wall, measurement_end)[key]
                for s in streams.values()
            )
            for key in [
                "setup_stream_errors",
                "stream_errors",
                "post_phase_stream_errors",
            ]
        },
        "stream_error_events": [
            {"session_id": s.actor.session, "epoch": epoch, "kind": kind}
            for s in streams.values()
            for epoch, kind in s.error_events
        ],
        "terminal_stream_errors": sum(
            s.terminal_error is not None for s in streams.values()
        ),
        "failed_tools": sum(s.failed_tools for s in streams.values()),
        "stream_cycles": sum(s.cycles for s in streams.values()),
        "replayed_frames": sum(s.replays for s in streams.values()),
        "forced_disconnects": sum(s.forced for s in streams.values()),
        "reconnect_ms": percentiles(
            [v for s in streams.values() for v in s.reconnect_ms]
        ),
        "environment": target.metadata,
    }
    report["queue"] = queue_metrics(results.rows, args.duration)
    report.update(verdict(report, args.ack_p95_ms))
    (args.output / "report.json").write_text(json.dumps(report, indent=2))
    if args.auth_state:
        from .local_setup import private_json

        private_json(args.auth_state, {a.name: a.headers for _, a in selected})
    print(json.dumps(report, indent=2))
    return report["passed"]


async def wait_for_audit(args, end_epoch):
    if not getattr(args, "audit_barrier", None):
        return end_epoch
    from .local_setup import atomic_json

    release = Path(args.audit_barrier)
    atomic_json(
        Path(str(release) + f".{args.shard_index}.done"), {"end_epoch": end_epoch}
    )
    deadline = time.monotonic() + args.duration + args.turn_timeout + args.setup_timeout
    while not release.exists():
        if time.monotonic() > deadline:
            raise TimeoutError("other generators did not finish the measured phase")
        await asyncio.sleep(0.05)
    # A fast generator keeps its streams alive while another drains its last
    # long turn. Use the global end for stream errors, before journal verification.
    return max(
        json.loads(Path(str(release) + f".{shard}.done").read_text())["end_epoch"]
        for shard in range(args.shards)
    )


def main(argv=None):
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("--manifest", required=True)
    p.add_argument(
        "--path", choices=["console", "agent-api", "work-chat"], required=True
    )
    p.add_argument("--users", type=int, default=1000)
    p.add_argument("--phase", choices=["steady", "burst"], default="steady")
    p.add_argument("--duration", type=float, default=1800)
    p.add_argument("--interval", type=float, default=60)
    p.add_argument("--turn-timeout", type=float, default=120)
    p.add_argument("--setup-concurrency", type=int, default=10)
    p.add_argument("--shards", type=int, default=1)
    p.add_argument("--shard-index", type=int, default=0)
    p.add_argument(
        "--mix",
        choices=["realistic", "chat-short", "file-research", "long-answer", "thinking"],
        default="realistic",
    )
    p.add_argument("--seed", default="capacity-1")
    p.add_argument("--run-id", default=None)
    p.add_argument(
        "--start-at",
        type=float,
        help="shared Unix start time for distributed generators",
    )
    p.add_argument("--ready-file", help="write a local setup readiness record")
    p.add_argument(
        "--audit-barrier",
        help="coordinator releases this shared file after all generators finish timing",
    )
    p.add_argument(
        "--start-file",
        help="coordinator writes a shared start epoch after all shards are ready",
    )
    p.add_argument("--setup-timeout", type=float, default=600)
    p.add_argument("--output", required=True)
    p.add_argument("--ca-bundle")
    p.add_argument(
        "--auth-state", help="private credential-state file outside published outputs"
    )
    p.add_argument("--background", action=argparse.BooleanOptionalAction, default=True)
    p.add_argument("--lost-ack-every", type=int, default=0)
    p.add_argument(
        "--fault-mode",
        action="store_true",
        help="separate recovery gates; retain all transient HTTP/poll/stream errors",
    )
    p.add_argument("--reconnect-at", type=float, default=-1)
    p.add_argument("--disconnect-seconds", type=float, default=2)
    p.add_argument("--audit-grace", type=float, default=2)
    p.add_argument("--audit-pages", type=int, default=100)
    p.add_argument("--ack-p95-ms", type=float, default=500)
    a = p.parse_args(argv)
    if (
        not 0 <= a.shard_index < a.shards
        or min(
            a.users,
            a.interval,
            a.duration,
            a.turn_timeout,
            a.setup_concurrency,
            a.audit_pages,
        )
        <= 0
    ):
        p.error("invalid capacity or timing options")
    if a.shards > 1 and (not a.run_id or (a.start_at is None and not a.start_file)):
        p.error("distributed shards require a shared --run-id and --start-at")
    a.run_id = a.run_id or str(uuid.uuid4())
    raise SystemExit(0 if asyncio.run(run(a)) else 1)
