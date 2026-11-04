"""Protocol and state tests for the stdlib MCP scale fixture."""

import json
import subprocess
import sys
import tempfile
import threading
import unittest
from http.server import ThreadingHTTPServer
from pathlib import Path
from urllib.request import Request, urlopen

from server import Fixture, Handler, build_catalog


class CatalogTests(unittest.TestCase):
    def test_catalog_is_deterministic_and_configurable(self):
        first = build_catalog(tools_count=600)
        second = build_catalog(tool_count=600)
        self.assertEqual(first, second)
        self.assertEqual(len(first), 600)
        self.assertGreaterEqual(len({item["_meta"]["fixture/domain"] for item in first}), 6)
        self.assertIn("rollback_deployment", {item["name"] for item in first})
        self.assertEqual(len(build_catalog(tool_count=2000)), 2000)

    def test_tools_list_paginates_without_duplicates(self):
        fixture = Fixture(tool_count=600, page_size=73)
        cursor = None
        seen = []
        while True:
            params = {} if cursor is None else {"cursor": cursor}
            response = fixture.dispatch({"jsonrpc": "2.0", "id": len(seen), "method": "tools/list", "params": params})
            page = response["result"]
            seen.extend(tool["name"] for tool in page["tools"])
            cursor = page.get("nextCursor")
            if cursor is None:
                break
        self.assertEqual(len(seen), 600)
        self.assertEqual(len(set(seen)), 600)
        self.assertEqual(seen, [tool["name"] for tool in fixture.catalog])

    def test_calls_validate_nested_and_flat_arguments(self):
        fixture = Fixture()
        for name, arguments in [
            ("list_deployments", {"service": "billing-api"}),
            ("list_deployments", {"service": "billing-api", "environment": "staging", "include_all": True}),
            ("create_ticket", {"details": {"title": "x", "description": "y", "service": "billing-api", "environment": "staging", "surprise": "field"}}),
            ("query_metric", {"service": "billing-api", "environment": "production", "metric": "error_rate", "time_range": "1h"}),
        ]:
            response = fixture.dispatch({"jsonrpc": "2.0", "id": 9, "method": "tools/call", "params": {"name": name, "arguments": arguments}})
            self.assertEqual(response["error"]["code"], -32602, response)
        valid = fixture.dispatch({"jsonrpc": "2.0", "id": 10, "method": "tools/call", "params": {"name": "create_ticket", "arguments": {"details": {"title": "Investigate", "description": "Check staging release", "service": "billing-api", "environment": "staging"}}}})
        self.assertEqual(valid["result"]["structuredContent"]["ticket_id"], "OPS-9001")

    def test_rollback_changes_only_the_target_and_persists(self):
        with tempfile.TemporaryDirectory() as directory:
            state_path = str(Path(directory) / "state.json")
            fixture = Fixture(state_file=state_path)
            response = fixture.dispatch({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "rollback_deployment", "arguments": {"deployment_id": "dep-bill-stg-2042", "reason": "Failed staging release after schema mismatch; restore previous version."}}})
            self.assertNotIn("error", response)
            self.assertEqual(response["result"]["structuredContent"]["deployment"]["version"], "3.8.0")
            reloaded = Fixture(state_file=state_path)
            staging = reloaded.state["deployments"]["dep-bill-stg-2042"]
            production = reloaded.state["deployments"]["dep-bill-prd-9911"]
            self.assertEqual(staging["status"], "rolled_back")
            self.assertEqual(staging["version"], "3.8.0")
            self.assertEqual(production["status"], "healthy")
            self.assertEqual(production["version"], "3.8.1")

    def test_stdio_transport_handles_initialize_notification_and_call(self):
        script = Path(__file__).with_name("server.py")
        requests = [
            {"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}},
            {"jsonrpc": "2.0", "method": "notifications/initialized"},
            {"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}},
        ]
        result = subprocess.run([sys.executable, str(script), "--transport", "stdio", "--tools", "30", "--page-size", "10"], input="\n".join(map(json.dumps, requests)) + "\n", text=True, capture_output=True, check=True)
        output = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertEqual(len(output), 2)
        self.assertEqual(output[0]["result"]["protocolVersion"], "2025-06-18")
        self.assertEqual(output[1]["result"]["tools"][0]["name"], build_catalog(tool_count=30)[0]["name"])
        self.assertEqual(output[1]["result"]["nextCursor"], "page:10")

    def test_http_streamable_transport_returns_202_for_notification(self):
        fixture = Fixture(tool_count=40, page_size=10)
        httpd = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        httpd.fixture = fixture
        thread = threading.Thread(target=httpd.serve_forever, daemon=True)
        thread.start()
        url = f"http://127.0.0.1:{httpd.server_port}/mcp"
        try:
            def post(value):
                request = Request(url, data=json.dumps(value).encode(), headers={"Content-Type": "application/json"}, method="POST")
                with urlopen(request) as response:
                    return response.status, response.headers, json.loads(response.read()) if response.status != 202 else None

            status, headers, init = post({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}})
            self.assertEqual(status, 200)
            self.assertEqual(init["result"]["protocolVersion"], "2025-06-18")
            self.assertEqual(headers["MCP-Protocol-Version"], "2025-06-18")
            status, _, body = post({"jsonrpc": "2.0", "method": "notifications/initialized"})
            self.assertEqual(status, 202)
            self.assertIsNone(body)
            status, _, body = post({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}})
            self.assertEqual(status, 200)
            self.assertEqual(len(body["result"]["tools"]), 10)
            self.assertEqual(body["result"]["nextCursor"], "page:10")
        finally:
            httpd.shutdown()
            httpd.server_close()
            thread.join(timeout=2)


if __name__ == "__main__":
    unittest.main()
