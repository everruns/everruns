"""End-to-end boundaries for the local two-tool MCP catalog gateway."""

from __future__ import annotations

import copy
from contextlib import ExitStack
import json
import os
import socket
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from http.server import ThreadingHTTPServer
from pathlib import Path
from urllib.error import URLError
from urllib.request import Request, urlopen

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))
sys.path.insert(0, str(ROOT / "fixture"))

from gateway import Backend, Catalog, MAX_RESULT_BYTES, dispatch, encoded, tool_result  # noqa: E402
from server import Fixture, Handler  # noqa: E402


class BackendServer:
    def __init__(self, tool_count: int = 60, page_size: int = 11):
        self.fixture = Fixture(tool_count=tool_count, page_size=page_size)
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.server.fixture = self.fixture
        self.thread = threading.Thread(target=lambda: self.server.serve_forever(poll_interval=0.02), daemon=True)
        self.thread.start()
        self.url = f"http://127.0.0.1:{self.server.server_port}/mcp"

    def close(self) -> None:
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=2)

    def __enter__(self) -> "BackendServer":
        return self

    def __exit__(self, *_args) -> None:
        self.close()


def rpc_request(url: str, request: dict) -> tuple[int, dict | None]:
    body = json.dumps(request).encode()
    req = Request(url, data=body, headers={"Content-Type": "application/json"}, method="POST")
    with urlopen(req, timeout=2) as response:
        raw = response.read()
        return response.status, json.loads(raw) if raw else None


def gateway_tool_call(catalog: Catalog, name: str, arguments: dict) -> dict:
    response = dispatch(catalog, {"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": arguments}})
    assert response is not None
    result = response["result"].get("structuredContent", {})
    if response["result"].get("isError"):
        raise AssertionError(result.get("error"))
    return result


class GatewayTests(unittest.TestCase):
    def test_server_description_selects_tools_with_identical_definitions(self):
        with BackendServer(40, 13) as left, BackendServer(40, 13) as right:
            catalog = Catalog([
                Backend("alpha", left.url, description="Payroll compensation and benefits"),
                Backend("beta", right.url, description="Geothermal drilling telemetry"),
            ])
            catalog.refresh()
            results = catalog.search("geothermal")["tools"]
            self.assertTrue(results)
            self.assertTrue(all(tool["server"] == "beta" for tool in results))
            self.assertTrue(all(tool["server_description"] == "Geothermal drilling telemetry"
                                for tool in results))
            result = results[0]
            catalog.backends["beta"].description = "Marine instrumentation"
            catalog.refresh()
            self.assertEqual(result["schema_digest"], catalog.tools[result["tool_id"]]["schema_digest"])
            self.assertEqual(catalog.search("geothermal")["tools"], [])
            self.assertTrue(catalog.search("marine")["tools"])

    def test_unauthorized_server_description_is_not_searchable(self):
        with BackendServer(40, 13) as backend:
            catalog = Catalog([
                Backend("alpha", backend.url, description="Geothermal drilling telemetry"),
                Backend("beta", backend.url, description="Payroll compensation and benefits"),
            ], allowed={"alpha::get_deployment"})
            catalog.refresh()
            result = catalog.search("payroll")
            self.assertEqual(result["tools"], [])
            self.assertEqual(result["available_servers"], ["alpha"])
            self.assertNotIn("Payroll", json.dumps(result))

    def test_search_byte_budget_includes_text_and_structured_content(self):
        catalog = Catalog([])
        tool = {"name": "rollback", "description": "rollback " + "x" * 19000,
                "tool_id": "ops::rollback", "inputSchema": {"type": "object"}}
        catalog.tools = {tool["tool_id"]: tool}
        result = catalog.search("rollback")
        self.assertEqual(result["tools"], [])
        self.assertEqual(result["omitted_for_byte_budget"], 1)
        self.assertLessEqual(len(encoded(tool_result(result))), MAX_RESULT_BYTES)

    def test_ten_server_topology_keeps_2000_tools_distinct(self):
        with ExitStack() as stack:
            servers = [stack.enter_context(BackendServer(200, 50)) for _ in range(10)]
            catalog = Catalog([Backend(f"team{i}", server.url) for i, server in enumerate(servers)])
            catalog.refresh()
            self.assertEqual(len(catalog.tools), 2000)
            scoped = catalog.search("get deployment", server="team9")
            self.assertIn("team9::get_deployment", {tool["tool_id"] for tool in scoped["tools"]})
            self.assertEqual(scoped["server_count"], 10)
            last = catalog.tools["team9::get_deployment"]
            catalog.call(last["tool_id"], last["schema_digest"], {"deployment_id": "dep-bill-stg-2042"})
            self.assertEqual([server.fixture.state["calls"] for server in servers], [0] * 9 + [1])

    def test_stdio_gateway_discloses_only_two_tools_and_searches_backend(self):
        with BackendServer(600, 100) as backend:
            requests = [
                {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}},
                {"jsonrpc": "2.0", "method": "notifications/initialized"},
                {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
                {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "catalog_search", "arguments": {"query": "rollback deployment"}}},
                {"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "catalog_search", "arguments": {"query": "geothermal"}}},
            ]
            result = subprocess.run([sys.executable, str(ROOT / "gateway.py"), "--transport", "stdio", "--server", f"platform={backend.url}",
                "--server-description", "platform=Geothermal drilling telemetry"],
                input="".join(json.dumps(request) + "\n" for request in requests), capture_output=True, text=True, timeout=10, check=True)
            responses = [json.loads(line) for line in result.stdout.splitlines()]
            self.assertEqual([response["id"] for response in responses], [1, 2, 3, 4])
            self.assertEqual(len(responses[1]["result"]["tools"]), 2)
            self.assertIn("platform::rollback_deployment", {tool["tool_id"] for tool in responses[2]["result"]["structuredContent"]["tools"]})
            by_server_description = responses[3]["result"]["structuredContent"]["tools"]
            self.assertTrue(by_server_description)
            self.assertTrue(all(tool["server_description"] == "Geothermal drilling telemetry"
                                for tool in by_server_description))

    def test_full_pagination_loads_600_and_2000_tools(self):
        for count in (600, 2000):
            with self.subTest(tool_count=count), BackendServer(count, page_size=37) as backend:
                catalog = Catalog([Backend("scale", backend.url)])
                catalog.refresh()
                self.assertEqual(len(catalog.tools), count)
                self.assertIn("scale::rollback_deployment", catalog.tools)
                self.assertEqual(len({tool["tool_id"] for tool in catalog.tools.values()}), count)

    def test_colliding_tool_names_are_qualified_and_route_to_the_right_server(self):
        with BackendServer(60, 13) as left, BackendServer(60, 17) as right:
            right.fixture.state["deployments"]["dep-bill-stg-2042"]["version"] = "9.9.9"
            catalog = Catalog([Backend("alpha", left.url), Backend("beta", right.url)])
            catalog.refresh()
            self.assertIn("alpha::get_deployment", catalog.tools)
            self.assertIn("beta::get_deployment", catalog.tools)
            found = catalog.search("get deployment details", limit=8)["tools"]
            ids = {tool["tool_id"] for tool in found}
            self.assertIn("alpha::get_deployment", ids)
            self.assertIn("beta::get_deployment", ids)
            a = catalog.tools["alpha::get_deployment"]
            b = catalog.tools["beta::get_deployment"]
            result_a = catalog.call(a["tool_id"], a["schema_digest"], {"deployment_id": "dep-bill-stg-2042"})
            result_b = catalog.call(b["tool_id"], b["schema_digest"], {"deployment_id": "dep-bill-stg-2042"})
            self.assertEqual(result_a["structuredContent"]["version"], "3.8.1")
            self.assertEqual(result_b["structuredContent"]["version"], "9.9.9")

    def test_repeated_pagination_cursor_is_rejected(self):
        with BackendServer(50, 10) as backend:
            original = backend.fixture.dispatch

            def looping(request):
                response = original(request)
                if request.get("method") == "tools/list" and response is not None:
                    response["result"]["nextCursor"] = "page:10"
                return response

            backend.fixture.dispatch = looping
            catalog = Catalog([Backend("loop", backend.url)])
            with self.assertRaisesRegex(ValueError, "repeated pagination cursor"):
                catalog.refresh()
            self.assertEqual(catalog.tools, {})

    def test_failed_refresh_does_not_install_a_partial_catalog(self):
        with BackendServer(40, 10) as first, BackendServer(30, 10) as second:
            catalog = Catalog([Backend("first", first.url), Backend("second", second.url)])
            catalog.refresh()
            previous = catalog.tools
            old_count = len(previous)
            original = second.fixture.dispatch

            def fail_listing(request):
                if request.get("method") == "tools/list":
                    return Fixture._error(request.get("id"), -32000, "temporarily unavailable")
                return original(request)

            second.fixture.dispatch = fail_listing
            with self.assertRaisesRegex(ValueError, "temporarily unavailable"):
                catalog.refresh()
            self.assertIs(catalog.tools, previous)
            self.assertEqual(len(catalog.tools), old_count)

    def test_filtered_search_and_direct_call_cannot_reach_unauthorized_tool(self):
        with BackendServer(50, 12) as backend:
            allowed = {"ops::get_deployment"}
            catalog = Catalog([Backend("ops", backend.url)], allowed=allowed)
            catalog.refresh()
            results = catalog.search("rollback failed deployment", limit=8)["tools"]
            self.assertNotIn("ops::rollback_deployment", {tool["tool_id"] for tool in results})
            forbidden = catalog.tools["ops::rollback_deployment"]
            with self.assertRaisesRegex(ValueError, "unavailable in current scope"):
                catalog.call(forbidden["tool_id"], forbidden["schema_digest"], {"deployment_id": "dep-bill-stg-2042", "reason": "test"})
            self.assertEqual(backend.fixture.state["calls"], 0)

    def test_revoked_allow_set_blocks_a_previously_discovered_tool(self):
        with BackendServer(40, 13) as backend:
            catalog = Catalog([Backend("ops", backend.url)], allowed={"ops::get_deployment"})
            catalog.refresh()
            tool = catalog.tools["ops::get_deployment"]
            catalog.call(tool["tool_id"], tool["schema_digest"], {"deployment_id": "dep-bill-stg-2042"})
            self.assertEqual(backend.fixture.state["calls"], 1)
            catalog.allowed = frozenset()
            with self.assertRaisesRegex(ValueError, "unavailable in current scope"):
                catalog.call(tool["tool_id"], tool["schema_digest"], {"deployment_id": "dep-bill-stg-2042"})
            self.assertEqual(backend.fixture.state["calls"], 1)

    def test_schema_digest_rejects_old_definition_after_refresh(self):
        with BackendServer(50, 10) as backend:
            catalog = Catalog([Backend("ops", backend.url)])
            catalog.refresh()
            previous = copy.deepcopy(catalog.tools["ops::get_deployment"])
            definition = next(tool for tool in backend.fixture.catalog if tool["name"] == "get_deployment")
            definition["inputSchema"]["properties"]["include_events"] = {"type": "boolean"}
            definition["inputSchema"]["required"] = ["deployment_id", "include_events"]
            backend.fixture.by_name["get_deployment"] = definition
            catalog.refresh()
            current = catalog.tools["ops::get_deployment"]
            self.assertNotEqual(previous["schema_digest"], current["schema_digest"])
            with self.assertRaisesRegex(ValueError, "stale tool definition"):
                catalog.call(previous["tool_id"], previous["schema_digest"], {"deployment_id": "dep-bill-stg-2042"})
            self.assertEqual(backend.fixture.state["calls"], 0)

    def test_invalid_arguments_are_rejected_before_backend_execution(self):
        with BackendServer(40, 9) as backend:
            catalog = Catalog([Backend("ops", backend.url)])
            catalog.refresh()
            tool = catalog.tools["ops::list_deployments"]
            with self.assertRaises(Exception):
                catalog.call(tool["tool_id"], tool["schema_digest"], {"service": "billing-api", "environment": "staging", "unrecognized": True})
            self.assertEqual(backend.fixture.state["calls"], 0)

    def test_deployment_rollback_through_http_gateway_changes_only_staging(self):
        with BackendServer(600, 57) as backend:
            gateway_port = self._free_port()
            command = [
                sys.executable, str(ROOT / "gateway.py"),
                "--server", f"ops={backend.url}", "--port", str(gateway_port),
            ]
            process = subprocess.Popen(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            url = f"http://127.0.0.1:{gateway_port}/mcp"
            try:
                self._wait_for_gateway(url, process)
                status, init = rpc_request(url, {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}})
                self.assertEqual(status, 200)
                self.assertEqual(init["result"]["protocolVersion"], "2025-06-18")
                _, listed = rpc_request(url, {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
                self.assertEqual({tool["name"] for tool in listed["result"]["tools"]}, {"catalog_search", "catalog_call"})
                _, searched = rpc_request(url, {"jsonrpc": "2.0", "id": 3, "method": "tools/call", "params": {"name": "catalog_search", "arguments": {"query": "rollback failed billing-api staging deployment", "limit": 8}}})
                matches = searched["result"]["structuredContent"]["tools"]
                rollback = next(tool for tool in matches if tool["tool_id"] == "ops::rollback_deployment")
                _, result = rpc_request(url, {"jsonrpc": "2.0", "id": 4, "method": "tools/call", "params": {"name": "catalog_call", "arguments": {"tool_id": rollback["tool_id"], "schema_digest": rollback["schema_digest"], "arguments": {"deployment_id": "dep-bill-stg-2042", "reason": "Failed 3.8.1 staging deployment; restore the recorded previous version."}}}})
                self.assertFalse(result["result"]["isError"])
                nested = result["result"]["structuredContent"]["deployment"]
                self.assertEqual(nested["status"], "rolled_back")
                self.assertEqual(nested["version"], "3.8.0")
                self.assertEqual(backend.fixture.state["deployments"]["dep-bill-prd-9911"]["status"], "healthy")
                self.assertEqual(backend.fixture.state["deployments"]["dep-bill-prd-9911"]["version"], "3.8.1")
            finally:
                process.terminate()
                try:
                    process.communicate(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.communicate()

    @staticmethod
    def _free_port() -> int:
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            return sock.getsockname()[1]

    @staticmethod
    def _wait_for_gateway(url: str, process: subprocess.Popen, timeout: float = 6.0) -> None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if process.poll() is not None:
                stdout, stderr = process.communicate()
                raise AssertionError(f"gateway exited early ({process.returncode}): {stdout}\n{stderr}")
            try:
                rpc_request(url, {"jsonrpc": "2.0", "id": 0, "method": "ping", "params": {}})
                return
            except (OSError, URLError):
                time.sleep(0.05)
        raise AssertionError("gateway did not start before timeout")


if __name__ == "__main__":
    unittest.main()
