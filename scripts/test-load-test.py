#!/usr/bin/env python3
"""Offline checks for the load driver's simulated-provider boundary and SSE startup."""
import asyncio
import argparse
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

import httpx

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


if __name__ == "__main__":
    unittest.main()
