"""Bounded SSE framing, cursor resumption and durable-event deduplication.

Projected streams legitimately have sequence gaps. Correctness is audited
against persisted events, rather than assuming every sequence is visible.
"""

from __future__ import annotations

import asyncio
import json
import time

import httpx
from .protocol import authenticated_request


async def frames(lines):
    kind, event_id, data, size = None, None, [], 0
    async for line in lines:
        size += len(line.encode()) + 1
        if size > 8 * 1024 * 1024:
            raise ValueError("SSE frame too large")
        if not line:
            if data:
                yield kind, event_id, json.loads("\n".join(data))
            kind, event_id, data, size = None, None, [], 0
        elif line.startswith("event:"):
            kind = line[6:].lstrip()
        elif line.startswith("id:"):
            event_id = line[3:].lstrip()
        elif line.startswith("data:"):
            data.append(line[5:].lstrip())


class Stream:
    def __init__(self, client, actor):
        self.client, self.actor = client, actor
        self.sequence = 0
        self.queue = asyncio.Queue(maxsize=10000)
        self.ready = asyncio.Event()
        self.connected = False
        self.connections = self.errors = self.replays = self.frames = 0
        self.forced = 0
        self.cycles = 0
        self.reconnect_ms = []
        self.inputs = {}  # only input ids/text; never store streamed token history
        self.terminal_error = None
        self.failed_tools = 0
        self.task = None
        self._last_closed = None

    def start(self):
        self.task = asyncio.create_task(self.read())
        return self.task

    async def stop(self):
        if self.task:
            self.task.cancel()
            await asyncio.gather(self.task, return_exceptions=True)
        self.connected = False

    async def reconnect(self, delay=0):
        self.forced += 1
        await self.stop()
        self.ready.clear()
        self._last_closed = time.perf_counter()
        await asyncio.sleep(delay)
        self.start()

    async def read(self):
        initial_retry = (
            1.5
            if self.actor.path == "work-chat"
            else 1
            if self.actor.path == "console"
            else 0.25
        )
        retry = initial_retry
        while True:
            graceful = False
            try:
                async with self.client.stream(
                    "GET",
                    self.actor.url(f"sessions/{self.actor.session}/sse"),
                    params={"after_sequence": self.sequence},
                    headers=self.actor.headers,
                    timeout=None,
                ) as response:
                    if response.status_code == 401 and self.actor.path == "console":
                        refreshed = await authenticated_request(
                            self.client, self.actor, "GET", self.actor.url("auth/me")
                        )
                        if refreshed.is_success:
                            continue
                    if response.status_code in {401, 403, 404}:
                        self.terminal_error = f"sse_http_{response.status_code}"
                        self.ready.set()
                        return
                    response.raise_for_status()
                    self.connections += 1
                    self.connected = True
                    self.ready.set()
                    if self._last_closed is not None:
                        self.reconnect_ms.append(
                            (time.perf_counter() - self._last_closed) * 1000
                        )
                        self._last_closed = None
                    retry = initial_retry
                    async for kind, event_id, event in frames(response.aiter_lines()):
                        self.frames += 1
                        if not isinstance(event, dict):
                            raise ValueError("invalid SSE envelope")
                        sequence = event.get("sequence")
                        if isinstance(sequence, int):
                            if sequence <= self.sequence:
                                self.replays += 1
                                continue
                            self.sequence = sequence
                        kind = event.get("type", kind)
                        if kind == "disconnecting":
                            graceful = True
                        if kind == "input.message":
                            message = event.get("data", {}).get("message", {})
                            text = "\n".join(
                                p.get("text", "")
                                for p in message.get("content", [])
                                if p.get("type") == "text"
                            )
                            self.inputs[message.get("id", event_id)] = text
                        if (
                            kind == "tool.completed"
                            and event.get("data", {}).get("success") is False
                        ):
                            self.failed_tools += 1
                        # Deltas are measured, not queued in unbounded token history.
                        if kind in {
                            "input.message",
                            "turn.started",
                            "turn.completed",
                            "turn.failed",
                            "turn.cancelled",
                            "output.message.completed",
                        } or (kind or "").endswith(".delta"):
                            if not self.queue.full():
                                self.queue.put_nowait(
                                    (kind, event, time.perf_counter())
                                )
                            else:
                                raise ValueError("SSE consumer fell behind")
                    if graceful:
                        self.cycles += 1
                    else:
                        self.errors += 1  # unsolicited EOF requires a reconnect
            except (httpx.HTTPError, ValueError, json.JSONDecodeError):
                if graceful:
                    self.cycles += 1
                else:
                    self.errors += 1
            finally:
                self.connected = False
            self._last_closed = self._last_closed or time.perf_counter()
            await asyncio.sleep(retry)
            retry = min(retry * 2, 10)
