#!/usr/bin/env python3
"""Local experiment: bounded MCP search/call over paginated backend catalogs.

Only loopback backends are accepted. Production belongs in the Everruns tool
registry and egress/auth boundary, rather than in this demonstration gateway.
"""
import argparse
import hashlib
import http.client
import json
import re
import sys
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit

import jsonschema

from retrieval import rank_bm25


def object_schema(properties, required):
    return {"type": "object", "properties": properties, "required": required,
            "additionalProperties": False}


GATEWAY_TOOLS = [
    {"name": "catalog_search",
     "description": "Find platform tools by intent. Search again with different words if needed. Returns qualified tool_id, schema_digest and authoritative inputSchema; use these with catalog_call. Use server to disambiguate namespaces; results list up to 16 authorized servers. Covers deployments, logs, incidents, tickets and platform administration.",
     "inputSchema": object_schema({
         "query": {"type": "string", "minLength": 1, "maxLength": 500},
         "server": {"type": "string", "description": "Optional exact server name from available_servers."},
         "limit": {"type": "integer", "minimum": 1, "maximum": 8, "default": 5}}, ["query"]),
     "annotations": {"readOnlyHint": True}},
    {"name": "catalog_call",
     "description": "Call an exact tool discovered by catalog_search. Pass its tool_id and schema_digest unchanged, and arguments matching its inputSchema. Operations may change fake platform state.",
     "inputSchema": object_schema({
         "tool_id": {"type": "string"}, "schema_digest": {"type": "string"},
         "arguments": {"type": "object", "additionalProperties": True}},
         ["tool_id", "schema_digest", "arguments"]),
     "annotations": {"readOnlyHint": False, "destructiveHint": True}},
]
MAX_CATALOG_BYTES = 16 * 1024 * 1024
MAX_RESULT_BYTES = 32 * 1024
MAX_PAGES = 256
MAX_TOOLS = 10000


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode()


class Backend:
    def __init__(self, name, url, description=""):
        parsed = urlsplit(url)
        if (not re.fullmatch(r"[A-Za-z][A-Za-z0-9_-]{0,63}", name) or parsed.scheme != "http"
                or parsed.hostname not in {"localhost", "127.0.0.1", "::1"}
                or parsed.username or parsed.password or parsed.query or parsed.fragment):
            raise ValueError("experiment backends must be named loopback HTTP endpoints")
        self.name, self.url, self.parsed = name, url, parsed
        if not isinstance(description, str) or len(description.encode()) > 4096:
            raise ValueError("server description must be a string of at most 4096 UTF-8 bytes")
        self.description = description
        self.session_id = None

    def rpc(self, method, params, notification=False):
        connection = http.client.HTTPConnection(self.parsed.hostname, self.parsed.port, timeout=15)
        headers = {"Content-Type": "application/json", "Accept": "application/json, text/event-stream",
                   "MCP-Protocol-Version": "2025-06-18"}
        if self.session_id:
            headers["Mcp-Session-Id"] = self.session_id
        try:
            request = {"jsonrpc": "2.0", "method": method, "params": params}
            if not notification:
                request["id"] = 1
            connection.request("POST", self.parsed.path or "/mcp", encoded(request), headers)
            response = connection.getresponse()
            body = response.read(MAX_CATALOG_BYTES + 1)
            if notification and response.status in {200, 202, 204}:
                return None
            if response.status != 200 or len(body) > MAX_CATALOG_BYTES:
                raise ValueError(f"backend response rejected: status={response.status}, bytes={len(body)}")
            self.session_id = response.getheader("Mcp-Session-Id", self.session_id)
            message = json.loads(body)
            if "error" in message:
                raise ValueError(str(message["error"]))
            return message["result"]
        finally:
            connection.close()

    def discover(self):
        self.rpc("initialize", {"protocolVersion": "2025-06-18", "capabilities": {},
                              "clientInfo": {"name": "mcp-scale-experiment", "version": "0.1"}})
        self.rpc("notifications/initialized", {}, notification=True)
        tools, seen, cursor, byte_count = [], set(), None, 0
        for _ in range(MAX_PAGES):
            result = self.rpc("tools/list", {"cursor": cursor} if cursor is not None else {})
            byte_count += len(encoded(result))
            tools.extend(result["tools"])
            if byte_count > MAX_CATALOG_BYTES or len(tools) > MAX_TOOLS:
                raise ValueError("catalog exceeds explicit experiment bounds")
            cursor = result.get("nextCursor")
            if cursor is None:
                return tools
            if not isinstance(cursor, str) or cursor in seen:
                raise ValueError("invalid or repeated pagination cursor")
            seen.add(cursor)
        raise ValueError("catalog exceeds page limit")


class Catalog:
    def __init__(self, backends, allowed=None):
        self.backends = {backend.name: backend for backend in backends}
        if len(self.backends) != len(backends):
            raise ValueError("duplicate server identity")
        # Trusted host configuration. A model cannot supply or widen this set.
        self.allowed = None if allowed is None else frozenset(allowed)
        self.tools = {}
        self.lock = threading.RLock()

    def permitted(self, tool_id):
        return self.allowed is None or tool_id in self.allowed

    def refresh(self):
        tools = {}
        for backend in self.backends.values():
            for definition in backend.discover():
                name = definition["name"]
                tool_id = f"{backend.name}::{name}"
                if tool_id in tools:
                    raise ValueError(f"duplicate tool identity: {tool_id}")
                jsonschema.Draft202012Validator.check_schema(definition["inputSchema"])
                tool = dict(definition, server=backend.name, tool_id=tool_id,
                            schema_digest=hashlib.sha256(encoded(definition)).hexdigest())
                if backend.description:
                    tool["server_description"] = backend.description
                tools[tool_id] = tool
                if len(tools) > MAX_TOOLS:
                    raise ValueError("aggregate catalog exceeds tool bound")
        if len(encoded(tools)) > MAX_CATALOG_BYTES:
            raise ValueError("aggregate catalog exceeds byte bound")
        with self.lock:
            self.tools = tools  # A failed refresh never installs a partial catalog.

    def search(self, query, limit=5, server=None):
        with self.lock:
            visible = [tool for key, tool in self.tools.items() if self.permitted(key)]
        servers = sorted({tool.get("server", "") for tool in visible})
        if server is not None:
            visible = [tool for tool in visible if tool.get("server") == server]
        ranked = rank_bm25(visible, query, limit)
        matches, omitted = [], 0
        def result(items, skipped):
            return {"tools": items, "omitted_for_byte_budget": skipped,
                    "available_servers": servers[:16], "server_count": len(servers),
                    "message": "Refine the query if the desired tool is absent. No match is not proof the capability is unavailable."}
        for tool in ranked:
            # MCP includes both text and structured content. Budget the actual
            # tool result, including that duplication, rather than just entries.
            if len(encoded(tool_result(result(matches + [tool], 8)))) > MAX_RESULT_BYTES:
                omitted += 1
            else:
                matches.append(tool)
        return result(matches, omitted)

    def call(self, tool_id, schema_digest, arguments):
        with self.lock:
            if not self.permitted(tool_id) or tool_id not in self.tools:
                raise ValueError("tool unavailable in current scope")
            tool = self.tools[tool_id]
            if tool["schema_digest"] != schema_digest:
                raise ValueError("stale tool definition: search again")
            jsonschema.Draft202012Validator(tool["inputSchema"]).validate(arguments)
            return self.backends[tool["server"]].rpc("tools/call", {
                "name": tool["name"], "arguments": arguments})


def tool_result(value, error=False):
    return {"content": [{"type": "text", "text": json.dumps(value)}],
            "structuredContent": value if isinstance(value, dict) else {"value": value}, "isError": error}


def dispatch(catalog, request):
    if "id" not in request:
        return None
    method, params = request.get("method"), request.get("params", {})
    try:
        if method == "initialize":
            result = {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}},
                      "serverInfo": {"name": "mcp-scale-catalog", "version": "0.1"}}
        elif method == "ping":
            result = {}
        elif method == "tools/list":
            result = {"tools": GATEWAY_TOOLS}
        elif method == "tools/call":
            name, arguments = params.get("name"), params.get("arguments", {})
            definition = next((tool for tool in GATEWAY_TOOLS if tool["name"] == name), None)
            if definition is None:
                raise ValueError("unknown catalog tool")
            jsonschema.validate(arguments, definition["inputSchema"])
            if name == "catalog_search":
                result = tool_result(catalog.search(**arguments))
            else:
                result = catalog.call(**arguments)
        else:
            return {"jsonrpc": "2.0", "id": request["id"], "error": {"code": -32601, "message": "Method not found"}}
    except (ValueError, KeyError, jsonschema.ValidationError) as error:
        result = tool_result({"error": str(error)[:1000]}, True)
    return {"jsonrpc": "2.0", "id": request["id"], "result": result}


def serve(catalog, port=8788):
    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            length = int(self.headers.get("Content-Length", "0"))
            if self.path != "/mcp" or not 0 < length <= 1024 * 1024:
                self.send_error(400)
                return
            try:
                response = dispatch(catalog, json.loads(self.rfile.read(length)))
            except (ValueError, TypeError):
                self.send_error(400)
                return
            body = encoded(response) if response is not None else b""
            self.send_response(200 if response is not None else 202)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_GET(self):
            self.send_error(405)  # No optional SSE notification stream in this experiment.

        def log_message(self, *_args):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    print(f"Catalog gateway: http://127.0.0.1:{server.server_port}/mcp, {len(catalog.tools)} tools behind 2 tools", flush=True)
    try:
        server.serve_forever()
    finally:
        server.server_close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", action="append", required=True, help="NAME=http://127.0.0.1:PORT/mcp; repeat for federation")
    parser.add_argument("--server-description", action="append", default=[],
                        help="NAME=DESCRIPTION; searchable configured MCP server metadata")
    parser.add_argument("--port", type=int, default=8788)
    parser.add_argument("--transport", choices=["http", "stdio"], default="http")
    parser.add_argument("--allow", action="append", help="Exact qualified tool id; omit for all fake tools")
    args = parser.parse_args()
    backends = [Backend(*server.split("=", 1)) for server in args.server]
    descriptions = {}
    for entry in args.server_description:
        name, separator, description = entry.partition("=")
        if not separator or name in descriptions:
            parser.error("--server-description requires one NAME=DESCRIPTION per server")
        descriptions[name] = description
    if set(descriptions) - {backend.name for backend in backends}:
        parser.error("--server-description names an unattached server")
    for backend in backends:
        if backend.name in descriptions:
            # Validate and bind metadata before the catalog's atomic refresh.
            backend.description = descriptions[backend.name]
            if len(backend.description.encode()) > 4096:
                parser.error("server description exceeds 4096 UTF-8 bytes")
    catalog = Catalog(backends, args.allow)
    catalog.refresh()
    if args.transport == "stdio":
        for line in sys.stdin:
            try:
                response = dispatch(catalog, json.loads(line))
            except (ValueError, TypeError):
                response = {"jsonrpc": "2.0", "id": None,
                            "error": {"code": -32700, "message": "Invalid JSON-RPC request"}}
            if response is not None:
                print(json.dumps(response, separators=(",", ":")), flush=True)
    else:
        serve(catalog, args.port)


if __name__ == "__main__":
    main()
