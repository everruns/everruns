#!/usr/bin/env python3
"""Minimal offline MCP stdio fixture for Framework acceptance tests."""

import json
import sys


for line in sys.stdin:
    try:
        request = json.loads(line)
    except json.JSONDecodeError:
        continue
    method = request.get("method", "")
    request_id = request.get("id")
    if method == "initialize":
        result = {
            "protocolVersion": "2025-06-18",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "framework-fixture", "version": "0"},
        }
    elif method == "tools/list":
        result = {
            "tools": [
                {
                    "name": "echo",
                    "description": "Echo a message",
                    "inputSchema": {
                        "type": "object",
                        "properties": {"message": {"type": "string"}},
                        "required": ["message"],
                    },
                }
            ],
            "nextCursor": None,
        }
    elif method == "notifications/initialized":
        continue
    else:
        result = {"tools": [], "nextCursor": None}
    if request_id is not None:
        print(
            json.dumps({"jsonrpc": "2.0", "id": request_id, "result": result}),
            flush=True,
        )
