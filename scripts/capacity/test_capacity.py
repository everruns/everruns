"""Offline failure oracles: a benchmark must fail rather than report false capacity."""

import asyncio
import json
from pathlib import Path
import tempfile
import unittest
from types import SimpleNamespace

import httpx

from .protocol import Actor, Target, check_url, load_target, preflight
from .stream import Stream, frames
from .run import Results, audit, post_message, verdict


def actor(path="console"):
    return Actor(
        "fixture",
        path,
        "https://localhost/api",
        {"Authorization": "Bearer never-log"},
        {"agent_id": "agent", "model_id": "model"},
        "org",
        "agent",
        "model",
        "channel",
        "session",
    )


def report():
    return {
        "users": 1000,
        "connected_users": 1000,
        "attempted": 2000,
        "admissions": 2000,
        "completed": 2000,
        "audit_errors": 0,
        "audited_inputs": 2000,
        "stream_missing_inputs": 0,
        "workload_audit_errors": 0,
        "workload_verified_turns": 2000,
        "reconnected_users": 0,
        "ack_ms": {"p95": 100},
        "poll_errors": {},
        "errors": {},
    }


class CapacityTests(unittest.IsolatedAsyncioTestCase):
    def test_a_shortened_failed_soak_cannot_qualify(self):
        r = report()
        r.update(phase="steady", duration_seconds=300, requested_duration_seconds=1800)
        self.assertFalse(verdict(r, 500)["passed"])
        self.assertFalse(verdict(r, 500)["checks"]["full_steady_duration_observed"])

    def test_proxy_config_contains_quoted_paths_and_never_key_contents(self):
        from unittest.mock import patch
        from .local_setup import Lab

        with tempfile.TemporaryDirectory() as temporary:
            lab = Lab(
                SimpleNamespace(
                    run_dir=str(Path(temporary) / "lab with spaces"), port_prefix=284
                )
            )
            lab.cert_key_path.write_text("private key contents must stay in this file")
            with patch.object(lab, "start"), patch.object(lab, "wait"):
                lab.start_host_and_proxy()
            path = lab.directory / "Caddyfile"
            config = path.read_text()
            self.assertIn(json.dumps(str(lab.cert_path)), config)
            self.assertIn(json.dumps(str(lab.cert_key_path)), config)
            self.assertNotIn(lab.cert_key_path.read_text(), config)
            self.assertNotIn(lab.chat_key, config)
            self.assertNotIn(lab.fixture_secret, config)
            self.assertEqual(path.stat().st_mode & 0o777, 0o600)

    def test_fast_fallback_and_missing_tools_cannot_pass_workload_gate(self):
        from .run import workload_valid

        terminal = {"tool_call_count": 4, "llm_call_count": 4, "duration_ms": 15000}
        output = {"content": [{"type": "text", "text": "word " * 600}]}
        self.assertTrue(workload_valid("file-research", terminal, output)[0])
        for changed in [
            {"tool_call_count": 0},
            {"duration_ms": 100},
            {"llm_call_count": 1},
        ]:
            self.assertFalse(
                workload_valid("file-research", {**terminal, **changed}, output)[0]
            )
        for shape in ["chat-short", "file-research", "long-answer", "thinking"]:
            self.assertFalse(
                workload_valid(
                    shape,
                    terminal,
                    {"content": [{"type": "text", "text": "Hello simulated response"}]},
                )[0]
            )

    def test_a_requested_reconnect_must_actually_recover_every_user(self):
        r = report()
        r["workload"] = {"reconnect_at_seconds": 10}
        self.assertFalse(verdict(r, 500)["passed"])
        r["reconnected_users"] = r["users"]
        self.assertTrue(verdict(r, 500)["passed"])

    async def test_work_chat_projection_correlates_input_and_terminal_turn(self):
        import time
        from .run import one_turn

        stream = SimpleNamespace(queue=asyncio.Queue())

        def respond(_request):
            for kind, data in [
                ("input.message", {"message": {"id": "old"}}),
                ("turn.started", {"input_message_id": "old", "turn_id": "old"}),
                ("input.message", {"message": {"id": "accepted"}}),
                ("turn.started", {"turn_id": "current"}),
                ("output.message.delta", {"message_id": "answer", "delta": "hi"}),
                ("turn.completed", {"turn_id": "old"}),
                ("turn.completed", {"turn_id": "current"}),
            ]:
                stream.queue.put_nowait(
                    (
                        kind,
                        {"data": data, "ts": "2026-01-01T00:00:00Z"},
                        time.perf_counter(),
                    )
                )
            return httpx.Response(201, json={"id": "accepted"})

        args = SimpleNamespace(
            seed="test",
            run_id="test",
            mix="chat-short",
            phase="burst",
            started=time.perf_counter(),
            turn_timeout=1,
            lost_ack_every=0,
            fault_mode=False,
        )
        with tempfile.TemporaryDirectory() as directory:
            results = Results(Path(directory))
            try:
                async with httpx.AsyncClient(
                    transport=httpx.MockTransport(respond)
                ) as client:
                    self.assertTrue(
                        await one_turn(
                            client, actor("work-chat"), stream, 0, 0, args, results
                        )
                    )
                self.assertTrue(results.rows[0]["completed"])
                self.assertIsNotNone(results.rows[0]["first_delta_ms"])
                self.assertFalse(results.errors)
            finally:
                results.handle.close()

    async def test_expired_console_cookie_refresh_is_serialized_and_preserves_intent(
        self,
    ):
        from .protocol import authenticated_request

        refreshes, requests = [], []

        async def respond(req):
            if req.url.path.endswith("/auth/refresh"):
                refreshes.append(req)
                await asyncio.sleep(0.01)
                return httpx.Response(
                    200,
                    headers=[
                        ("set-cookie", "access_token=new; Path=/"),
                        ("set-cookie", "refresh_token=next; Path=/"),
                    ],
                )
            requests.append(req)
            return httpx.Response(
                200 if "access_token=new" in req.headers["Cookie"] else 401
            )

        a = actor()
        a.headers = {"Cookie": "access_token=old; refresh_token=old"}
        async with httpx.AsyncClient(transport=httpx.MockTransport(respond)) as client:
            responses = await asyncio.gather(
                *(
                    authenticated_request(
                        client,
                        a,
                        "POST",
                        a.url("intent"),
                        json={"same": True},
                        headers={"Idempotency-Key": "retained"},
                    )
                    for _ in range(2)
                )
            )
        self.assertTrue(all(r.status_code == 200 for r in responses))
        self.assertEqual(len(refreshes), 1)
        self.assertTrue(
            all(r.headers["Idempotency-Key"] == "retained" for r in requests)
        )
        self.assertEqual(len({r.content for r in requests}), 1)

    def test_queue_trend_distinguishes_growth_from_insufficient_samples(self):
        from .run import queue_metrics

        self.assertIsNone(queue_metrics([], 60)["stable"])
        rows = [{"start_seconds": i, "pickup_ms": 100} for i in range(60)]
        self.assertTrue(queue_metrics(rows, 60)["stable"])
        for row in rows[40:]:
            row["pickup_ms"] = 5000
        self.assertFalse(queue_metrics(rows, 60)["stable"])

    def test_merge_uses_raw_samples_and_rejects_missing_or_reused_shards(self):
        from .merge import merge

        base = {
            **report(),
            "run_id": "same",
            "path": "console",
            "phase": "burst",
            "total_users": 2,
            "shards": 2,
            "interval_seconds": 30,
            "background_polls": True,
            "environment": {},
            "fault_mode": False,
            "requested_duration_seconds": 30,
            "workload": {},
            "start_epoch": 100,
            "duration_seconds": 5,
            "identity_fingerprints": [],
            "started_users": 1,
            "scheduled_users": 1,
            "planned_attempts": 1,
            "failed_tools": 0,
            "auth_refreshes": 0,
            "stream_cycles": 0,
            "terminal_stream_errors": 0,
            "stream_connections": 1,
            "stream_errors": 0,
            "replayed_frames": 0,
            "forced_disconnects": 0,
            "http_status": {},
            "http_versions": {},
            "poll_requests": 0,
        }
        with tempfile.TemporaryDirectory() as temporary:
            directories = [Path(temporary) / str(i) for i in range(2)]
            for i, directory in enumerate(directories):
                directory.mkdir()
                (directory / "report.json").write_text(
                    json.dumps(
                        {
                            **base,
                            "shard_index": i,
                            "identity_fingerprints": [str(i)],
                            "start_epoch": 100 + i * 0.1,
                        }
                    )
                )
                (directory / "turns.jsonl").write_text(
                    json.dumps(
                        {
                            "marker": str(i),
                            "start_seconds": 0,
                            "ack_ms": i * 100,
                            "completed": True,
                            "admitted": True,
                            "input_received_ms": 0,
                            "turn_ms": 20,
                        }
                    )
                    + "\n"
                )
            result = merge(directories)
            self.assertEqual(result["ack_ms"]["mean"], 50)
            self.assertEqual(result["client_observed_peak_unfinished_turns"], 1)
            self.assertAlmostEqual(result["duration_seconds"], 5.1)
            row = json.loads((directories[0] / "turns.jsonl").read_text())
            row["completed"] = False
            (directories[0] / "turns.jsonl").write_text(json.dumps(row) + "\n")
            self.assertEqual(
                merge(directories)["client_observed_peak_unfinished_turns"], 2
            )
            with self.assertRaisesRegex(ValueError, "missing or duplicated"):
                merge(directories[:1])
            row = json.loads((directories[1] / "report.json").read_text())
            row["identity_fingerprints"] = ["0"]
            (directories[1] / "report.json").write_text(json.dumps(row))
            with self.assertRaisesRegex(ValueError, "reused"):
                merge(directories)

    def test_profile_routes_are_the_real_product_paths(self):
        a = actor()
        self.assertEqual(
            a.send({"message": {}}, "same-key"),
            (
                "https://localhost/api/v1/commands/create_message",
                {
                    "params": {
                        "session_id": "session",
                        "client_message_id": "same-key",
                        "message": {},
                    }
                },
            ),
        )
        for path in ["agent-api", "work-chat"]:
            self.assertEqual(
                actor(path).send({"message": {}}, "same-key"),
                ("https://localhost/api/sessions/session/messages", {"message": {}}),
            )
        self.assertEqual([n for n, _ in actor("work-chat").polls()], [4, 10])

    def test_failed_capacity_never_passes_its_gate(self):
        self.assertTrue(verdict(report(), 500)["passed"])
        for key, value in [
            ("connected_users", 999),
            ("admissions", 1990),
            ("completed", 1999),
            ("audit_errors", 1),
            ("workload_audit_errors", 1),
            ("workload_verified_turns", 1999),
            ("audited_inputs", 1999),
            ("stream_missing_inputs", 1),
            ("poll_errors", {"429": 1}),
            ("errors", {"setup": 1}),
            ("ack_ms", {"p95": 501}),
            ("arrival_lag_ms", {"p95": 101}),
        ]:
            r = report()
            r[key] = value
            self.assertFalse(verdict(r, 500)["passed"], key)

    def test_manifest_is_private_and_requires_authentication_and_isolation(self):
        data = {
            "isolated": True,
            "management_url": "https://localhost/api",
            "management_headers": {"Authorization": "private"},
            "agent_id": "agent",
            "model_id": "model",
            "actors": [
                {
                    "name": "user",
                    "path": "console",
                    "base": "https://localhost/api",
                    "headers": {"Cookie": "private"},
                    "org": "org",
                    "create": {"agent_id": "agent"},
                }
            ],
        }
        with tempfile.TemporaryDirectory() as directory:
            p = Path(directory) / "credentials.json"
            p.write_text(json.dumps(data))
            p.chmod(0o644)
            with self.assertRaisesRegex(ValueError, "0600"):
                load_target(p)
            p.chmod(0o600)
            self.assertNotIn("private", json.dumps(load_target(p).metadata))
            data["isolated"] = False
            p.write_text(json.dumps(data))
            with self.assertRaisesRegex(ValueError, "isolated"):
                load_target(p)
        for url in [
            "https://app.everruns.com/api",
            "https://user:pass@localhost",
            "https://localhost?token=secret",
            "http://remote.example",
        ]:
            with self.assertRaises(ValueError):
                check_url(url)

    async def test_every_tenant_and_actual_agent_model_is_checked_before_work(self):
        calls = []

        def respond(req):
            calls.append(req)
            org = req.headers["X-Org-Id"]
            if "/models/" in req.url.path:
                return httpx.Response(
                    200, json={"enabled": True, "provider_id": "provider"}
                )
            if "/providers/" in req.url.path:
                return httpx.Response(
                    200, json={"provider_type": "openai" if org == "bad" else "llmsim"}
                )
            return httpx.Response(200, json={"default_model_id": "model"})

        a, b = actor(), actor()
        b.org = "bad"
        target = Target(
            [a, b],
            "https://localhost/api",
            {"Authorization": "control"},
            "model",
            "agent",
        )
        async with httpx.AsyncClient(transport=httpx.MockTransport(respond)) as client:
            with self.assertRaisesRegex(ValueError, "llmsim"):
                await preflight(client, target, [a, b])
        self.assertTrue(all(req.method == "GET" for req in calls))
        self.assertEqual({r.headers["X-Org-Id"] for r in calls}, {"org", "bad"})

    async def test_console_requires_explicit_simulator_override_while_api_uses_agent_default(
        self,
    ):
        def respond(req):
            if "/models/" in req.url.path:
                return httpx.Response(
                    200, json={"enabled": True, "provider_id": "provider"}
                )
            if "/providers/" in req.url.path:
                return httpx.Response(200, json={"provider_type": "llmsim"})
            return httpx.Response(200, json={"default_model_id": None})

        a = actor()
        target = Target([a], "https://localhost/api", {}, "model", "agent")
        async with httpx.AsyncClient(transport=httpx.MockTransport(respond)) as client:
            await preflight(client, target, [a])
            a.create.pop("model_id")
            with self.assertRaisesRegex(ValueError, "verified agent/model"):
                await preflight(client, target, [a])
            with self.assertRaisesRegex(ValueError, "verified llmsim model"):
                await preflight(client, target, [actor("agent-api")])

    async def test_lost_ack_reuses_both_body_and_key_for_all_paths(self):
        for path in ["console", "agent-api", "work-chat"]:
            calls = []

            def respond(req):
                calls.append(req)
                msg = {"id": "message"}
                return httpx.Response(
                    200, json={"output": msg} if path == "console" else msg
                )

            with tempfile.TemporaryDirectory() as directory:
                results = Results(Path(directory))
                async with httpx.AsyncClient(
                    transport=httpx.MockTransport(respond)
                ) as client:
                    result = await post_message(
                        client, actor(path), {"message": {}}, "same-key", results, True
                    )
                results.handle.close()
            self.assertEqual(result["id"], "message")
            self.assertEqual(len(calls), 2)
            self.assertEqual(calls[0].content, calls[1].content)
            self.assertEqual(
                [r.headers["Idempotency-Key"] for r in calls], ["same-key"] * 2
            )

    async def test_audit_catches_duplicated_unacknowledged_inputs_and_missing_stream_echo(
        self,
    ):
        def event(i, text):
            return {
                "type": "input.message",
                "sequence": i,
                "data": {
                    "message": {
                        "id": str(i),
                        "content": [
                            {"type": "text", "text": text + " [[llmsim:chat-short]]"}
                        ],
                    }
                },
            }

        target = Target([], "https://localhost/api", {}, "model", "agent")
        stream = SimpleNamespace(inputs={})
        async with httpx.AsyncClient(
            transport=httpx.MockTransport(
                lambda _: httpx.Response(
                    200, json={"data": [event(1, "uncertain"), event(2, "uncertain")]}
                )
            )
        ) as client:
            errors, missing, count = await audit(
                client, target, actor(), {"uncertain": False}, stream, 10
            )
        self.assertEqual((errors, missing, count), (1, 1, 2))

    async def test_audit_uses_backward_input_pagination_not_latest_N_forward_cursor(
        self,
    ):
        requests = []

        def event(i):
            return {
                "type": "input.message",
                "sequence": i,
                "data": {
                    "message": {
                        "id": str(i),
                        "content": [{"type": "text", "text": f"m{i} shape"}],
                    }
                },
            }

        def respond(req):
            requests.append(req)
            self.assertNotIn("types", req.url.params)
            self.assertNotIn("after_sequence", req.url.params)
            rows = (
                [event(i) for i in range(2, 1002)]
                if "before_sequence" not in req.url.params
                else [event(1)]
            )
            return httpx.Response(200, json={"data": rows})

        expected = {f"m{i}": True for i in range(1, 1002)}
        stream = SimpleNamespace(inputs={str(i): f"m{i} shape" for i in range(1, 1002)})
        target = Target([], "https://localhost/api", {}, "model", "agent")
        async with httpx.AsyncClient(transport=httpx.MockTransport(respond)) as client:
            self.assertEqual(
                await audit(client, target, actor(), expected, stream, 10), (0, 0, 1001)
            )
        self.assertEqual(requests[1].url.params["before_sequence"], "2")

    async def test_complete_capacity_profile_audits_real_admissions_after_lost_ack(
        self,
    ):
        from unittest.mock import patch
        from .run import run

        queues, histories, cached = {}, {}, {}

        class Body(httpx.AsyncByteStream):
            def __init__(self, queue):
                self.queue = queue

            async def __aiter__(self):
                while True:
                    yield await self.queue.get()

        async def respond(req):
            path = req.url.path
            if "/models/" in path:
                return httpx.Response(
                    200, json={"enabled": True, "provider_id": "provider"}
                )
            if "/providers/" in path:
                return httpx.Response(200, json={"provider_type": "llmsim"})
            if "/agents/" in path:
                return httpx.Response(200, json={"default_model_id": "model"})
            if path.endswith("/auth/me"):
                return httpx.Response(200, json={"id": req.headers["Cookie"]})
            if path.endswith("/sessions"):
                session = "session_" + str(len(queues))
                queues[session] = asyncio.Queue()
                histories[session] = []
                return httpx.Response(201, json={"id": session})
            if path.endswith("/sse"):
                return httpx.Response(200, stream=Body(queues[path.split("/")[-2]]))
            if path.endswith("/events"):
                types = req.url.params.get("types", "").split(",")
                return httpx.Response(
                    200,
                    json={
                        "data": [
                            e
                            for e in histories[path.split("/")[-2]]
                            if not types[0] or e["type"] in types
                        ]
                    },
                )
            if path.endswith("/commands/create_message"):
                key = req.headers["Idempotency-Key"]
                if key not in cached:
                    params = json.loads(req.content)["params"]
                    session = params["session_id"]
                    message = {
                        "id": "message_" + key,
                        "content": params["message"]["content"],
                    }
                    cached[key] = {"output": message}
                    events = [
                        {
                            "id": f"event_{key}_{i}",
                            "sequence": i,
                            "type": kind,
                            "ts": f"2026-01-01T00:00:0{i}Z",
                            "data": data,
                        }
                        for i, (kind, data) in enumerate(
                            [
                                ("input.message", {"message": message}),
                                (
                                    "turn.started",
                                    {"input_message_id": message["id"], "turn_id": key},
                                ),
                                ("output.message.delta", {"turn_id": key}),
                                (
                                    "output.message.completed",
                                    {
                                        "message": {
                                            "id": "final_" + key,
                                            "content": [
                                                {"type": "text", "text": "word " * 40}
                                            ],
                                        }
                                    },
                                ),
                                (
                                    "turn.completed",
                                    {
                                        "turn_id": key,
                                        "final_message_id": "final_" + key,
                                        "duration_ms": 1500,
                                        "tool_call_count": 0,
                                        "llm_call_count": 1,
                                    },
                                ),
                            ],
                            1,
                        )
                    ]
                    histories[session].extend(events)
                    for event in events:
                        queues[session].put_nowait(
                            (
                                "event: "
                                + event["type"]
                                + "\ndata: "
                                + json.dumps(event)
                                + "\n\n"
                            ).encode()
                        )
                return httpx.Response(200, json=cached[key])
            return httpx.Response(200, json={})

        transport = httpx.MockTransport(respond)
        actual = httpx.AsyncClient

        def client(**kwargs):
            kwargs.pop("http2", None)
            return actual(transport=transport, **kwargs)

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest = root / "private.json"
            manifest.write_text(
                json.dumps(
                    {
                        "isolated": True,
                        "management_url": "https://localhost/api",
                        "management_headers": {},
                        "model_id": "model",
                        "agent_id": "agent",
                        "actors": [
                            {
                                "name": str(i),
                                "org": "org",
                                "path": "console",
                                "base": "https://localhost/api",
                                "headers": {"Cookie": str(i)},
                                "create": {"agent_id": "agent", "model_id": "model"},
                            }
                            for i in range(2)
                        ],
                    }
                )
            )
            manifest.chmod(0o600)
            args = SimpleNamespace(
                manifest=str(manifest),
                path="console",
                users=2,
                shards=1,
                shard_index=0,
                output=str(root / "results"),
                ca_bundle=None,
                setup_concurrency=2,
                turn_timeout=2,
                background=False,
                phase="burst",
                duration=1,
                interval=30,
                seed="test",
                run_id="test",
                lost_ack_every=1,
                reconnect_at=-1,
                disconnect_seconds=0,
                audit_grace=0,
                audit_pages=10,
                ack_p95_ms=1000,
                ready_file=None,
                start_file=None,
                start_at=None,
                fault_mode=False,
                auth_state=None,
                setup_timeout=5,
                mix="chat-short",
            )
            with (
                patch("capacity.run.httpx.AsyncClient", client),
                patch("builtins.print"),
            ):
                self.assertTrue(await run(args))
            result = json.loads((root / "results/report.json").read_text())
            self.assertEqual(
                (result["admissions"], result["completed"], result["audited_inputs"]),
                (2, 2, 2),
            )
            self.assertEqual(len(cached), 2)
            self.assertEqual(result["http_status"]["200"], 4)
            args.output = str(root / "steady-results")
            args.phase, args.duration, args.interval = "steady", 0.05, 0.08
            with (
                patch("capacity.run.httpx.AsyncClient", client),
                patch("builtins.print"),
            ):
                self.assertTrue(await run(args))
            steady = json.loads((root / "steady-results/report.json").read_text())
            self.assertGreaterEqual(steady["duration_seconds"], args.duration)
            self.assertEqual(steady["planned_attempts"], 2)

    def test_local_fixture_strips_provider_credentials_but_preserves_proxy_guards(self):
        from .local_setup import clean_environment

        data = {
            "OPENAI_API_KEY": "paid",
            "DEFAULT_ANTHROPIC_API_KEY": "paid",
            "UTILITY_OPENAI_API_KEY": "paid",
            "AWS_SECRET_ACCESS_KEY": "paid",
            "DOPPLER_TOKEN": "private",
            "HTTP_PROXY": "http://proxy",
            "HTTPS_PROXY": "http://proxy",
            "NO_PROXY": "internal",
            "CODEX_PROXY_GUARD": "retained",
        }
        output = clean_environment(data)
        self.assertEqual(
            set(output), {"HTTP_PROXY", "HTTPS_PROXY", "NO_PROXY", "CODEX_PROXY_GUARD"}
        )

    def test_resource_gate_distinguishes_finished_generators_and_planned_restart(self):
        from .resources import summarize

        row = {
            "epoch": 10,
            "sample_gap": 2,
            "processes": {
                "server": {"rss_bytes": 100, "cpu_seconds": 1},
                "generator-0": {"rss_bytes": 100, "cpu_seconds": 1},
            },
            "database": None,
        }
        last = {
            "epoch": 12,
            "sample_gap": 2,
            "processes": {
                "server": {"missing": True},
                "generator-0": {"missing": True},
            },
            "database": None,
        }
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "resources.jsonl"
            path.write_text(json.dumps(row) + "\n" + json.dumps(last) + "\n")
            self.assertFalse(summarize(path, 10, 12, 1000)["passed"])
            self.assertTrue(summarize(path, 10, 12, 1000, {"server"})["passed"])
            self.assertFalse(summarize(path, 10, 12, 150, {"server"})["passed"])
            self.assertFalse(summarize(path, 10, 100, 1000, {"server"})["passed"])

    def test_macos_sampler_distinguishes_disappeared_process_from_ps_failure(self):
        from subprocess import CalledProcessError, CompletedProcess
        from unittest.mock import patch
        from .monitor import process_sample

        with (
            patch("platform.system", return_value="Darwin"),
            patch("capacity.monitor.Path.exists", return_value=False),
            patch("capacity.monitor.subprocess.run") as run,
        ):
            run.side_effect = CalledProcessError(1, "ps", output="", stderr="")
            with self.assertRaises(ProcessLookupError):
                process_sample(12345678)
            run.side_effect = CalledProcessError(2, "ps", output="", stderr="error")
            with self.assertRaises(CalledProcessError):
                process_sample(12345678)
            run.side_effect = None
            run.return_value = CompletedProcess("ps", 0, "1:02.5 512\n", "")
            sample = process_sample(12345678)
            self.assertEqual(sample["cpu_seconds"], 62.5)
            self.assertEqual(sample["rss_bytes"], 512 * 1024)


class StreamTests(unittest.IsolatedAsyncioTestCase):
    async def test_multiline_frames_and_noncontiguous_sequences(self):
        async def lines():
            for line in [
                "event: input.message",
                "id: event_1",
                'data: {"sequence": 3,',
                'data: "type":"input.message"}',
                "",
                "event: turn.completed",
                'data: {"sequence": 8}',
                "",
            ]:
                yield line

        output = [x async for x in frames(lines())]
        self.assertEqual([x[2]["sequence"] for x in output], [3, 8])

    async def test_oversized_frame_is_rejected(self):
        async def lines():
            yield "data: " + ("x" * (8 * 1024 * 1024))

        with self.assertRaisesRegex(ValueError, "large"):
            [x async for x in frames(lines())]

    async def test_resume_cursor_deduplicates_replayed_events(self):
        requests = []

        class Body(httpx.AsyncByteStream):
            async def __aiter__(self):
                yield b'event: input.message\ndata: {"type":"input.message","sequence":5,"data":{"message":{"id":"message","content":[]}}}\n\n'
                await asyncio.Event().wait()

        def respond(req):
            requests.append(req)
            return httpx.Response(200, stream=Body())

        async with httpx.AsyncClient(transport=httpx.MockTransport(respond)) as client:
            stream = Stream(client, actor())
            stream.start()
            await asyncio.wait_for(stream.ready.wait(), 1)
            while stream.sequence != 5:
                await asyncio.sleep(0)
            await stream.reconnect()
            await asyncio.wait_for(stream.ready.wait(), 1)
            while stream.replays != 1:
                await asyncio.sleep(0)
            await stream.stop()
        self.assertEqual(requests[1].url.params["after_sequence"], "5")
        self.assertEqual(len(stream.inputs), 1)

    async def test_auth_rejection_never_reconnects_or_counts_as_connected(self):
        for status in [401, 403, 404]:
            async with httpx.AsyncClient(
                transport=httpx.MockTransport(lambda _: httpx.Response(status))
            ) as client:
                stream = Stream(client, actor())
                stream.start()
                await asyncio.wait_for(stream.task, 1)
                self.assertEqual(stream.connections, 0)
                self.assertEqual(stream.terminal_error, f"sse_http_{status}")
