#!/usr/bin/env python3
"""Offline checks for the load driver's simulated-provider boundary and SSE startup."""
import asyncio
import argparse
from contextlib import redirect_stdout
import importlib.util
import io
import json
import random
import tempfile
from pathlib import Path
import unittest
from unittest.mock import patch

import httpx

from capacity.test_capacity import CapacityTests, StreamTests

spec = importlib.util.spec_from_file_location("load_test", Path(__file__).with_name("load-test.py"))
driver = importlib.util.module_from_spec(spec)
spec.loader.exec_module(driver)


class LoadDriverTests(unittest.IsolatedAsyncioTestCase):
    async def test_run_rejects_real_provider_before_creating_work(self):
        requests = []

        def respond(request):
            requests.append((request.method, request.url.path))
            if request.url.path.endswith("/models/model"):
                return httpx.Response(200, json={"provider_id": "provider", "enabled": True})
            return httpx.Response(200, json={"provider_type": "openai"})

        client = httpx.AsyncClient(transport=httpx.MockTransport(respond))
        args = argparse.Namespace(api="http://test/api", model="model", mix="chat-short")
        with patch.object(driver.httpx, "AsyncClient", return_value=client):
            with self.assertRaisesRegex(ValueError, "enabled llmsim"):
                await driver.run(args)
        self.assertTrue(client.is_closed)
        self.assertEqual(requests, [("GET", "/api/v1/models/model"), ("GET", "/api/v1/providers/provider")])

    async def test_real_and_disabled_models_are_rejected_before_work(self):
        for provider_type, enabled, accepted in (("openai", True, False), ("llmsim", False, False), ("llmsim", True, True)):
            with self.subTest(provider_type=provider_type, enabled=enabled):
                def respond(request):
                    if request.url.path == "/api/v1/models/model":
                        return httpx.Response(200, json={"provider_id": "provider", "enabled": enabled})
                    self.assertEqual(request.url.path, "/api/v1/providers/provider")
                    return httpx.Response(200, json={"provider_type": provider_type})

                async with httpx.AsyncClient(transport=httpx.MockTransport(respond)) as client:
                    if accepted:
                        await driver.validate_simulated_model(client, "http://test/api", "model")
                    else:
                        with self.assertRaisesRegex(ValueError, "enabled llmsim"):
                            await driver.validate_simulated_model(client, "http://test/api", "model")

    async def test_failed_sse_response_does_not_mark_session_connected(self):
        async with httpx.AsyncClient(transport=httpx.MockTransport(lambda _: httpx.Response(502))) as client:
            session = driver.Session(client, "http://test/api", "session")
            try:
                await asyncio.sleep(0)
                self.assertFalse(session.connected.is_set())
                self.assertEqual(session.stream_errors, 1)
            finally:
                session.reader.cancel()
                with self.assertRaises(asyncio.CancelledError):
                    await session.reader


class FakeApi:
    """An in-memory API: each posted message plays a short turn on the session's streams."""

    def __init__(self, drop_every=None):
        self.streams, self.events, self.requests = {}, {}, []
        self.drop_every, self.posts = drop_every, 0

    async def handle(self, request):
        path, method = request.url.path, request.method
        self.requests.append((method, path))
        if path.startswith("/api/v1/models/"):
            return httpx.Response(200, json={"provider_id": "provider", "enabled": True})
        if path.startswith("/api/v1/providers/"):
            return httpx.Response(200, json={"provider_type": "llmsim"})
        if path == "/api/v1/harnesses":
            return httpx.Response(200, json={"data": [{"id": "harness", "name": "generic"}]})
        if path == "/api/v1/agents":
            self.agent = json.loads(request.content)
            return httpx.Response(200, json={"id": "agent"})
        if path == "/api/v1/sessions":
            sid = f"session-{len(self.events)}"
            self.events[sid], self.streams[sid] = [], []
            return httpx.Response(200, json={"id": sid})
        sid = path.split("/")[4]
        if "/fs/" in path:
            return httpx.Response(200, json={})
        if method == "PATCH":
            return httpx.Response(200, json={"id": sid})
        if path.endswith("/sse"):
            queue = asyncio.Queue()
            self.streams[sid].append(queue)

            async def body():
                try:
                    while True:
                        yield (await queue.get()).encode()
                finally:
                    self.streams[sid].remove(queue)

            return httpx.Response(200, stream=FakeStream(body()))
        if path.endswith("/messages"):
            self.posts += 1
            text = json.loads(request.content)["message"]["content"][0]["text"]
            log = self.events[sid]
            if not (self.drop_every and self.posts % self.drop_every == 0):
                log.append({"sequence": len(log) + 1, "type": "input.message", "data": {"text": text}})
            for name in ("output.message.delta", "session.idled"):
                for queue in list(self.streams[sid]):
                    queue.put_nowait(f"event: {name}\ndata: {{}}\n\n")
            return httpx.Response(200, json={})
        if path.endswith("/events"):
            after = int(request.url.params["after_sequence"])
            return httpx.Response(200, json={"data": [e for e in self.events[sid] if e["sequence"] > after]})
        raise AssertionError(f"unexpected {method} {path}")


class FakeStream(httpx.AsyncByteStream):
    def __init__(self, gen):
        self.gen = gen

    async def __aiter__(self):
        async for chunk in self.gen:
            yield chunk

    async def aclose(self):
        await self.gen.aclose()


def fake_args(**overrides):
    args = {"api": "http://test/api", "model": "model", "mix": "chat-short", "sessions": 3,
            "messages": 3, "think": 0, "ramp": 0, "seed": "s", "turn_timeout": 5}
    args.update(overrides)
    return argparse.Namespace(**args)


class EndToEndTests(unittest.IsolatedAsyncioTestCase):
    async def run_fake(self, api, **overrides):
        out = io.StringIO()
        real = httpx.AsyncClient

        def client(**kwargs):
            kwargs.pop("limits", None)
            return real(transport=httpx.MockTransport(api.handle), **kwargs)

        with patch.object(driver.httpx, "AsyncClient", side_effect=client), redirect_stdout(out):
            passed = await driver.run(fake_args(**overrides))
        return passed, json.loads(out.getvalue())

    async def test_verified_run_with_tabs_switches_and_capabilities_passes(self):
        api = FakeApi()
        passed, result = await self.run_fake(
            api, tabs=2, switch_model="other", switch_every=1, verify=2, capabilities="bashkit_shell,subagents"
        )
        self.assertTrue(passed, result)
        self.assertEqual(result["turns"], 9)
        self.assertEqual(result["switches"], 6)
        self.assertEqual(result["integrity"], {"checked": 2, "failures": [], "failure_count": 0})
        self.assertEqual(api.agent["capabilities"], [{"ref": "bashkit_shell"}, {"ref": "subagents"}])
        self.assertIn(("GET", "/api/v1/models/other"), api.requests)

    async def test_lost_messages_fail_the_integrity_check(self):
        passed, result = await self.run_fake(FakeApi(drop_every=4), verify=3)
        self.assertFalse(passed)
        self.assertGreater(result["integrity"]["failure_count"], 0)
        self.assertIn("persisted 0 times", result["integrity"]["failures"][0]["problem"])

    async def test_soak_reopens_streams_and_herds_join(self):
        api = FakeApi()
        passed, result = await self.run_fake(
            api, sessions=2, duration=1.2, idle=(0.3, 0.4), reopen_after=0.2,
            herd_every=0.1, herd_fraction=1.0, file_kb=(2, 4),
        )
        self.assertTrue(passed, result)
        self.assertIsNone(result["messages"])
        self.assertGreater(result["reloads"]["count"], 0)
        self.assertGreater(result["herd_turns"], 0)
        self.assertGreater(result["turns"], 2)

    async def test_abandoned_sessions_send_and_leave(self):
        passed, result = await self.run_fake(FakeApi(), abandon=1.0, verify=3)
        self.assertTrue(passed, result)
        self.assertEqual(result["abandoned"], 3)
        self.assertEqual(result["turns"], 0)

    async def test_baselines_round_trip_and_catch_regressions(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "base.json"
            passed, _ = await self.run_fake(FakeApi(), save_baseline=str(path))
            self.assertTrue(passed)
            passed, result = await self.run_fake(FakeApi(), baseline=str(path))
            self.assertTrue(passed, result)
            base = json.loads(path.read_text())
            base["turn_p95"] = 0
            path.write_text(json.dumps(base))
            regressions = driver.compare_baseline({"turn_p95": 500}, base, 0.2)
            self.assertEqual(regressions, [{"metric": "turn_p95", "baseline": 0, "now": 500}])


class HelperTests(unittest.TestCase):
    def test_lognormal_hits_its_median_and_tail(self):
        rng = random.Random(1)
        draws = sorted(driver.lognormal(rng, 6, 60) for _ in range(20000))
        self.assertAlmostEqual(draws[10000], 6, delta=0.5)
        self.assertAlmostEqual(draws[19800], 60, delta=10)
        self.assertLessEqual(draws[-1], 240)

    def test_ranges_must_be_ordered_and_positive(self):
        self.assertEqual(driver.parse_range("6,60"), (6.0, 60.0))
        self.assertEqual(driver.parse_range("5"), (5.0, 5.0))
        for bad in ("0,5", "10,5", "-1,2"):
            with self.assertRaises(argparse.ArgumentTypeError):
                driver.parse_range(bad)

    def test_research_notes_reach_the_requested_size(self):
        self.assertEqual(driver.research_notes(), driver.RESEARCH_NOTES)
        self.assertAlmostEqual(len(driver.research_notes(64)), 64 * 1024, delta=200)

    def test_poisson_arrivals_are_seeded_with_the_ramp_mean(self):
        args = argparse.Namespace(arrival="poisson", ramp=2.0, sessions=4000, seed="x")
        first, again = driver.arrivals(args), driver.arrivals(args)
        self.assertEqual(first, again)
        self.assertEqual(first[0], 0)
        self.assertAlmostEqual(first[-1] / len(first), 2.0, delta=0.15)
        args.arrival = "even"
        self.assertEqual(driver.arrivals(args)[:3], [0, 2.0, 4.0])

    def test_integrity_flags_missing_duplicate_and_reordered_messages(self):
        def event(seq, n):
            return {"sequence": seq, "type": "input.message", "data": {"text": f"m ck={n}"}}

        sent = ["a1", "b2", "c3"]
        good = [event(1, "a1"), {"sequence": 2, "type": "turn.completed"}, event(3, "b2"), event(4, "c3")]
        self.assertEqual(driver.check_integrity(good, sent), [])
        self.assertEqual(driver.check_integrity(good[:3], sent), ["ck=c3 persisted 0 times"])
        self.assertEqual(driver.check_integrity(good + [event(5, "c3")], sent), ["ck=c3 persisted 2 times"])
        reordered = [event(1, "b2"), event(2, "a1"), event(3, "c3")]
        self.assertEqual(driver.check_integrity(reordered, sent), ["messages persisted out of order"])

    def test_baseline_ignores_noise_and_missing_metrics(self):
        base = {"turn_p50": 1000, "turn_p95": 100, "first_p50": None}
        self.assertEqual(driver.compare_baseline({"turn_p50": 1150, "turn_p95": 140, "first_p50": 9}, base, 0.2), [])
        self.assertEqual(
            driver.compare_baseline({"turn_p50": 1300}, base, 0.2),
            [{"metric": "turn_p50", "baseline": 1000, "now": 1300}],
        )


if __name__ == "__main__":
    unittest.main()
