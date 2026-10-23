#!/usr/bin/env python3
"""Deterministic, stateful MCP fixture for scale evaluations.

Only Python's standard library is required. Run as Streamable HTTP or stdio:

    python3 server.py --transport http --host 127.0.0.1 --port 8787 \
      --tools 600 --page-size 100 --audit calls.jsonl
    python3 server.py --transport stdio --tools 600 --page-size 100

The catalog is also importable by an evaluator via ``build_catalog`` and
``dispatch``. The HTTP implementation intentionally speaks the 2025-06-18
JSON-RPC request/response shape, including ``nextCursor`` pagination.
"""

from __future__ import annotations

import argparse
import copy
import hashlib
import json
import os
import sys
import threading
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

PROTOCOL_VERSION = "2025-06-18"
SERVER_INFO = {"name": "everruns-mcp-scale-fixture", "version": "1.0.0"}
PAGE_CURSOR_PREFIX = "page:"


def _object(properties: dict[str, Any], required: list[str]) -> dict[str, Any]:
    return {
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": False,
    }


def _string(description: str, enum: list[str] | None = None) -> dict[str, Any]:
    result: dict[str, Any] = {"type": "string", "description": description}
    if enum is not None:
        result["enum"] = enum
    return result


def _integer(description: str, minimum: int = 1, maximum: int = 100) -> dict[str, Any]:
    return {
        "type": "integer",
        "description": description,
        "minimum": minimum,
        "maximum": maximum,
    }


def _tool(name: str, description: str, schema: dict[str, Any], domain: str) -> dict[str, Any]:
    return {
        "name": name,
        "description": description,
        "inputSchema": schema,
        "annotations": {"title": name.replace("_", " ").title(), "readOnlyHint": False},
        "_meta": {"fixture/domain": domain},
    }


def _base_tools() -> list[dict[str, Any]]:
    svc = _string("Service slug, for example billing-api.")
    env = _string("Deployment environment.", ["dev", "staging", "prod"])
    deployment_id = _string("Exact deployment identifier returned by list_deployments.")
    incident_id = _string("Incident identifier, for example INC-427.")
    ticket_id = _string("Ticket identifier, for example OPS-8421.")
    query = _string("Terms to match against indexed records.")
    time_range = _string("Lookback interval.", ["15m", "1h", "6h", "24h", "7d"])
    return [
        _tool("list_deployments", "Find deployments by service, environment, and status. Use this first to identify the exact deployment before inspecting or changing it.", _object({"service": svc, "environment": env, "status": _string("Optional deployment state.", ["healthy", "failed", "rolling_back", "rolled_back"])}, ["service", "environment"]), "deployments"),
        _tool("get_deployment", "Read one deployment, including current version, previous version, health checks, and rollback eligibility.", _object({"deployment_id": deployment_id}, ["deployment_id"]), "deployments"),
        _tool("rollback_deployment", "Roll a failed deployment back to its recorded previous version. Requires the exact deployment id and an explicit reason; this mutates deployment state.", _object({"deployment_id": deployment_id, "reason": _string("Short operational reason for the rollback.")}, ["deployment_id", "reason"]), "deployments"),
        _tool("search_logs", "Search service logs by environment, query, and time range. Results include timestamps, severity, deployment id, and trace id.", _object({"service": svc, "environment": env, "query": query, "time_range": time_range}, ["service", "environment", "query", "time_range"]), "observability"),
        _tool("get_log_entry", "Fetch a single log entry by its stable log id.", _object({"log_id": _string("Log entry identifier returned by search_logs.")}, ["log_id"]), "observability"),
        _tool("query_metric", "Read a named service metric for an environment and time range.", _object({"service": svc, "environment": env, "metric": _string("Metric name, such as error_rate or request_latency_ms."), "time_range": time_range}, ["service", "environment", "metric", "time_range"]), "observability"),
        _tool("list_incidents", "List incidents filtered by service and environment.", _object({"service": svc, "environment": env, "status": _string("Optional incident state.", ["open", "investigating", "mitigated", "closed"])}, ["service", "environment"]), "incidents"),
        _tool("get_incident", "Read the incident timeline, owner, impact, and linked ticket.", _object({"incident_id": incident_id}, ["incident_id"]), "incidents"),
        _tool("update_incident", "Change an incident status or append an operator note. Provide at least one update field.", _object({"incident_id": incident_id, "status": _string("New incident state.", ["open", "investigating", "mitigated", "closed"]), "note": _string("Optional note appended to the timeline.")}, ["incident_id"]), "incidents"),
        _tool("get_ticket", "Read a ticket, including owner, status, and linked incident.", _object({"ticket_id": ticket_id}, ["ticket_id"]), "tickets"),
        _tool("search_tickets", "Find tickets by service, environment, or matching text.", _object({"query": query}, ["query"]), "tickets"),
        _tool("add_ticket_comment", "Append a comment to an existing ticket. This mutates the ticket timeline.", _object({"ticket_id": ticket_id, "comment": _string("Comment to append.")}, ["ticket_id", "comment"]), "tickets"),
        _tool("create_ticket", "Create a new operational ticket with a structured service and environment scope.", _object({"details": _object({"title": _string("Short actionable title."), "description": _string("Observed symptoms and requested follow-up."), "service": svc, "environment": env, "owner": _string("Optional ticket owner.")}, ["title", "description", "service", "environment"])}, ["details"]), "tickets"),
        _tool("get_service", "Read service ownership, on-call team, and operational metadata.", _object({"service": svc}, ["service"]), "service_catalog"),
        _tool("list_services", "List services, optionally filtered by owning team.", _object({"team": _string("Optional owning team name.")}, []), "service_catalog"),
        _tool("get_runbook", "Fetch the runbook for a service and operational task.", _object({"service": svc, "topic": _string("Runbook topic, such as rollback or elevated-errors.")}, ["service", "topic"]), "runbooks"),
        _tool("search_runbooks", "Search runbooks by operational query.", _object({"query": query}, ["query"]), "runbooks"),
        _tool("list_change_windows", "List approved change windows for a service and environment.", _object({"service": svc, "environment": env}, ["service", "environment"]), "change_management"),
        _tool("get_change", "Read a change record and its approval status.", _object({"change_id": _string("Change identifier, for example CHG-1234.")}, ["change_id"]), "change_management"),
    ]


DISTRACTOR_DOMAINS = (
    "billing", "identity", "storage", "messaging", "search", "analytics",
    "inventory", "checkout", "notifications", "fraud", "reporting", "edge",
)
DISTRACTOR_ACTIONS = ("inspect", "compare", "summarize", "validate", "lookup", "preview")


def build_catalog(tool_count: int = 600, *, tools_count: int | None = None) -> list[dict[str, Any]]:
    """Build a stable catalog with 19 realistic tools and named distractors."""
    if tools_count is not None:
        tool_count = tools_count
    if tool_count < 19:
        raise ValueError("tool_count must be at least 19 (the fixture has 19 real tools)")
    tools = _base_tools()
    i = 0
    while len(tools) < tool_count:
        domain = DISTRACTOR_DOMAINS[i % len(DISTRACTOR_DOMAINS)]
        action = DISTRACTOR_ACTIONS[(i // len(DISTRACTOR_DOMAINS)) % len(DISTRACTOR_ACTIONS)]
        suffix = i // (len(DISTRACTOR_DOMAINS) * len(DISTRACTOR_ACTIONS)) + 1
        name = f"{action}_{domain}_record_{suffix:03d}"
        tools.append(_tool(
            name,
            f"{action.title()} a {domain} operation record by its exact key. Returns the matching record summary and detail level.",
            _object({"record_key": _string("Synthetic record key."), "detail_level": _string("Requested detail.", ["brief", "standard", "full"])}, ["record_key"]),
            domain,
        ))
        i += 1
    # Spread the useful operations across every page rather than making the
    # first page a hand-picked shortlist. SHA-256 keeps order stable across
    # Python processes, machines, and PYTHONHASHSEED values.
    return sorted(tools, key=lambda tool: hashlib.sha256(tool["name"].encode()).digest())


def initial_state() -> dict[str, Any]:
    return {
        "deployments": {
            "dep-bill-stg-2042": {
                "deployment_id": "dep-bill-stg-2042", "service": "billing-api",
                "environment": "staging", "version": "3.8.1", "previous_version": "3.8.0",
                "status": "failed", "rollback_eligible": True,
                "health_checks": {"http_5xx_rate": 0.184, "ready_replicas": 1, "desired_replicas": 3},
            },
            "dep-bill-prd-9911": {
                "deployment_id": "dep-bill-prd-9911", "service": "billing-api",
                "environment": "prod", "version": "3.8.1", "previous_version": "3.8.0",
                "status": "healthy", "rollback_eligible": False,
                "health_checks": {"http_5xx_rate": 0.001, "ready_replicas": 6, "desired_replicas": 6},
            },
        },
        "incidents": {
            "INC-427": {
                "incident_id": "INC-427", "service": "billing-api", "environment": "staging",
                "status": "investigating", "severity": "SEV-2", "owner": "Ava Li",
                "summary": "Elevated 5xx responses after staging release 3.8.1.",
                "linked_ticket": "OPS-8421", "timeline": ["09:14 UTC: alert triggered after dep-bill-stg-2042."]
            }
        },
        "tickets": {
            "OPS-8421": {
                "ticket_id": "OPS-8421", "title": "Investigate billing-api staging 5xx after 3.8.1",
                "owner": "Ava Li", "status": "in_progress", "linked_incident": "INC-427", "comments": []
            }
        },
        "ticket_sequence": 9000,
        "calls": 0,
    }


def _validate(schema: dict[str, Any], value: Any, path: str = "arguments") -> str | None:
    """Small JSON Schema subset used by the fixture's object/enum contracts."""
    expected = schema.get("type")
    if expected == "object":
        if not isinstance(value, dict):
            return f"{path} must be an object"
        for key in schema.get("required", []):
            if key not in value:
                return f"{path}.{key} is required"
        properties = schema.get("properties", {})
        if schema.get("additionalProperties") is False:
            extras = sorted(set(value) - set(properties))
            if extras:
                return f"{path} has unknown field(s): {', '.join(extras)}"
        for key, item in value.items():
            if key in properties:
                error = _validate(properties[key], item, f"{path}.{key}")
                if error:
                    return error
        return None
    if expected == "string":
        if not isinstance(value, str):
            return f"{path} must be a string"
        if value == "":
            return f"{path} must not be empty"
    elif expected == "integer":
        if isinstance(value, bool) or not isinstance(value, int):
            return f"{path} must be an integer"
        if value < schema.get("minimum", value) or value > schema.get("maximum", value):
            return f"{path} is outside the allowed range"
    elif expected == "number":
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            return f"{path} must be a number"
    if "enum" in schema and value not in schema["enum"]:
        return f"{path} must be one of: {', '.join(map(str, schema['enum']))}"
    return None


class Fixture:
    """Catalog, JSON-RPC dispatch, and optional durable state/audit sinks."""

    def __init__(self, tool_count: int = 600, page_size: int = 100,
                 state_file: str | None = None, audit_file: str | None = None):
        if page_size < 1:
            raise ValueError("page_size must be positive")
        self.catalog = build_catalog(tool_count)
        self.by_name = {tool["name"]: tool for tool in self.catalog}
        self.page_size = page_size
        self.state_file = Path(state_file) if state_file else None
        self.audit_file = Path(audit_file) if audit_file else None
        self.lock = threading.RLock()
        self.state = self._load_state()

    def _load_state(self) -> dict[str, Any]:
        if self.state_file and self.state_file.exists():
            try:
                return json.loads(self.state_file.read_text(encoding="utf-8"))
            except (OSError, json.JSONDecodeError) as exc:
                raise ValueError(f"could not load state file {self.state_file}: {exc}") from exc
        return initial_state()

    def _persist(self) -> None:
        if not self.state_file:
            return
        self.state_file.parent.mkdir(parents=True, exist_ok=True)
        tmp = self.state_file.with_suffix(self.state_file.suffix + ".tmp")
        tmp.write_text(json.dumps(self.state, indent=2, sort_keys=True) + "\n", encoding="utf-8")
        os.replace(tmp, self.state_file)

    def _audit(self, record: dict[str, Any]) -> None:
        if not self.audit_file:
            return
        self.audit_file.parent.mkdir(parents=True, exist_ok=True)
        record = {"at": datetime.now(timezone.utc).isoformat(), **record}
        with self.audit_file.open("a", encoding="utf-8") as stream:
            stream.write(json.dumps(record, sort_keys=True) + "\n")

    def _tools_list(self, params: dict[str, Any]) -> dict[str, Any]:
        cursor = params.get("cursor")
        offset = 0
        if cursor is not None:
            if not isinstance(cursor, str) or not cursor.startswith(PAGE_CURSOR_PREFIX):
                raise RpcFault(-32602, "invalid pagination cursor")
            try:
                offset = int(cursor[len(PAGE_CURSOR_PREFIX):])
            except ValueError as exc:
                raise RpcFault(-32602, "invalid pagination cursor") from exc
        if offset < 0 or offset > len(self.catalog):
            raise RpcFault(-32602, "pagination cursor out of range")
        end = min(offset + self.page_size, len(self.catalog))
        result: dict[str, Any] = {"tools": copy.deepcopy(self.catalog[offset:end])}
        if end < len(self.catalog):
            result["nextCursor"] = f"{PAGE_CURSOR_PREFIX}{end}"
        self._audit({"method": "tools/list", "cursor": cursor, "count": end - offset})
        return result

    def _tool_call(self, params: dict[str, Any]) -> dict[str, Any]:
        name = params.get("name")
        if not isinstance(name, str) or name not in self.by_name:
            self._audit({"method": "tools/call", "tool": name, "ok": False, "error": "unknown tool"})
            raise RpcFault(-32602, f"unknown tool: {name}")
        arguments = params.get("arguments", {})
        error = _validate(self.by_name[name]["inputSchema"], arguments)
        if error:
            self._audit({"method": "tools/call", "tool": name, "arguments": arguments, "validation_error": error})
            raise RpcFault(-32602, error)
        with self.lock:
            self.state["calls"] = self.state.get("calls", 0) + 1
            try:
                result = self._execute(name, arguments)
                self._persist()
            except RpcFault as fault:
                self._audit({"method": "tools/call", "tool": name, "arguments": arguments, "ok": False, "error": str(fault)})
                raise
            self._audit({"method": "tools/call", "tool": name, "arguments": arguments, "ok": True})
        return {"content": [{"type": "text", "text": json.dumps(result, sort_keys=True)}], "structuredContent": result, "isError": False}

    def _execute(self, name: str, args: dict[str, Any]) -> dict[str, Any]:
        deployments = self.state["deployments"]
        incidents = self.state["incidents"]
        tickets = self.state["tickets"]
        if name == "list_deployments":
            rows = [d for d in deployments.values() if d["service"] == args["service"] and d["environment"] == args["environment"] and ("status" not in args or d["status"] == args["status"])]
            return {"deployments": copy.deepcopy(rows), "count": len(rows)}
        if name == "get_deployment":
            item = deployments.get(args["deployment_id"])
            if item is None:
                raise RpcFault(-32602, f"deployment not found: {args['deployment_id']}")
            return copy.deepcopy(item)
        if name == "rollback_deployment":
            item = deployments.get(args["deployment_id"])
            if item is None:
                raise RpcFault(-32602, f"deployment not found: {args['deployment_id']}")
            if not item["rollback_eligible"] or item["status"] != "failed":
                raise RpcFault(-32000, "rollback refused: deployment is not an eligible failed deployment")
            old_version = item["version"]
            item["version"] = item["previous_version"]
            item["previous_version"] = old_version
            item["status"] = "rolled_back"
            item["rollback_eligible"] = False
            item["rollback_reason"] = args["reason"]
            item["health_checks"] = {"http_5xx_rate": 0.002, "ready_replicas": 3, "desired_replicas": 3}
            return {"deployment": copy.deepcopy(item), "message": f"Rolled back {item['deployment_id']} to {item['version']}"}
        if name == "search_logs":
            logs = [
                {"log_id": "log-7701", "timestamp": "2026-10-18T09:14:02Z", "severity": "error", "service": "billing-api", "environment": "staging", "deployment_id": "dep-bill-stg-2042", "trace_id": "tr-55a", "message": "payment_authorization: upstream ledger timeout after schema v4 migration"},
                {"log_id": "log-7702", "timestamp": "2026-10-18T09:14:05Z", "severity": "error", "service": "billing-api", "environment": "staging", "deployment_id": "dep-bill-stg-2042", "trace_id": "tr-55b", "message": "HTTP 500: column account_region does not exist"},
            ]
            hits = [x for x in logs if x["service"] == args["service"] and x["environment"] == args["environment"] and args["query"].lower() in x["message"].lower()]
            return {"entries": hits, "count": len(hits), "time_range": args["time_range"]}
        if name == "get_log_entry":
            items = self._execute("search_logs", {"service": "billing-api", "environment": "staging", "query": "", "time_range": "1h"})["entries"]
            item = next((x for x in items if x["log_id"] == args["log_id"]), None)
            if item is None:
                raise RpcFault(-32602, f"log entry not found: {args['log_id']}")
            return item
        if name == "query_metric":
            value = 0.002 if args["environment"] == "staging" and deployments["dep-bill-stg-2042"]["status"] == "rolled_back" else 0.184
            return {"service": args["service"], "environment": args["environment"], "metric": args["metric"], "value": value, "unit": "ratio" if "rate" in args["metric"] else "ms", "time_range": args["time_range"]}
        if name == "list_incidents":
            rows = [x for x in incidents.values() if x["service"] == args["service"] and x["environment"] == args["environment"] and ("status" not in args or x["status"] == args["status"])]
            return {"incidents": copy.deepcopy(rows), "count": len(rows)}
        if name == "get_incident":
            item = incidents.get(args["incident_id"])
            if item is None:
                raise RpcFault(-32602, f"incident not found: {args['incident_id']}")
            return copy.deepcopy(item)
        if name == "update_incident":
            item = incidents.get(args["incident_id"])
            if item is None:
                raise RpcFault(-32602, f"incident not found: {args['incident_id']}")
            changed = False
            if "status" in args:
                item["status"] = args["status"]
                changed = True
            if "note" in args:
                item["timeline"].append(args["note"])
                changed = True
            if not changed:
                raise RpcFault(-32602, "provide status and/or note")
            return copy.deepcopy(item)
        if name == "get_ticket":
            item = tickets.get(args["ticket_id"])
            if item is None:
                raise RpcFault(-32602, f"ticket not found: {args['ticket_id']}")
            return copy.deepcopy(item)
        if name == "search_tickets":
            query = args["query"].lower()
            rows = [x for x in tickets.values() if query in json.dumps(x).lower()]
            return {"tickets": copy.deepcopy(rows), "count": len(rows)}
        if name == "add_ticket_comment":
            item = tickets.get(args["ticket_id"])
            if item is None:
                raise RpcFault(-32602, f"ticket not found: {args['ticket_id']}")
            item["comments"].append(args["comment"])
            return copy.deepcopy(item)
        if name == "create_ticket":
            self.state["ticket_sequence"] += 1
            ticket_id = f"OPS-{self.state['ticket_sequence']}"
            details = args["details"]
            item = {"ticket_id": ticket_id, **copy.deepcopy(details), "status": "open", "comments": []}
            tickets[ticket_id] = item
            return copy.deepcopy(item)
        if name == "get_service":
            if args["service"] != "billing-api":
                raise RpcFault(-32602, f"service not found: {args['service']}")
            return {"service": "billing-api", "team": "Payments Platform", "owner": "Ava Li", "on_call": "payments-oncall", "repo": "services/billing-api"}
        if name == "list_services":
            rows = [{"service": "billing-api", "team": "Payments Platform"}, {"service": "identity-api", "team": "Identity"}, {"service": "ledger-writer", "team": "Payments Platform"}]
            if "team" in args:
                rows = [x for x in rows if x["team"] == args["team"]]
            return {"services": rows, "count": len(rows)}
        if name == "get_runbook":
            return {"service": args["service"], "topic": args["topic"], "steps": ["Confirm the failing environment and deployment id.", "Inspect health checks and recent error logs.", "If the deployment is failed and rollback_eligible is true, roll back that exact deployment.", "Verify the deployment status and error rate after rollback."], "requires_approval": False}
        if name == "search_runbooks":
            return {"matches": [{"title": "Rollback a failed deployment", "service": "billing-api", "topic": "rollback"}], "query": args["query"]}
        if name == "list_change_windows":
            return {"windows": [{"change_id": "CHG-551", "service": args["service"], "environment": args["environment"], "status": "approved", "starts_at": "2026-10-18T09:00:00Z", "ends_at": "2026-10-18T10:00:00Z"}], "count": 1}
        if name == "get_change":
            if args["change_id"] != "CHG-551":
                raise RpcFault(-32602, f"change not found: {args['change_id']}")
            return {"change_id": "CHG-551", "status": "approved", "summary": "Staging billing-api release 3.8.1", "owner": "Ava Li", "environment": "staging"}
        return {"tool": name, "record_key": args["record_key"], "detail_level": args.get("detail_level", "standard"), "domain": self.by_name[name]["_meta"]["fixture/domain"]}

    def dispatch(self, request: dict[str, Any]) -> dict[str, Any] | None:
        """Dispatch one JSON-RPC request. Notifications return ``None``."""
        if not isinstance(request, dict) or request.get("jsonrpc") != "2.0":
            return self._error(request.get("id") if isinstance(request, dict) else None, -32600, "invalid JSON-RPC request")
        method = request.get("method")
        request_id = request.get("id")
        params = request.get("params") or {}
        if not isinstance(params, dict):
            return self._error(request_id, -32602, "params must be an object")
        if method == "notifications/initialized":
            return None
        if method == "initialize":
            requested = params.get("protocolVersion", PROTOCOL_VERSION)
            version = requested if requested in ("2025-03-26", PROTOCOL_VERSION) else PROTOCOL_VERSION
            return {"jsonrpc": "2.0", "id": request_id, "result": {"protocolVersion": version, "capabilities": {"tools": {"listChanged": False}}, "serverInfo": SERVER_INFO}}
        if method == "ping":
            result = {}
        elif method == "tools/list":
            try:
                result = self._tools_list(params)
            except RpcFault as fault:
                return self._error(request_id, fault.code, str(fault))
        elif method == "tools/call":
            try:
                result = self._tool_call(params)
            except RpcFault as fault:
                return self._error(request_id, fault.code, str(fault))
        else:
            return self._error(request_id, -32601, f"method not found: {method}")
        if "id" not in request:
            return None
        return {"jsonrpc": "2.0", "id": request_id, "result": result}

    @staticmethod
    def _error(request_id: Any, code: int, message: str) -> dict[str, Any]:
        return {"jsonrpc": "2.0", "id": request_id, "error": {"code": code, "message": message}}


class RpcFault(Exception):
    def __init__(self, code: int, message: str):
        super().__init__(message)
        self.code = code


class Handler(BaseHTTPRequestHandler):
    server_version = "EverrunsMcpFixture/1.0"
    protocol_version = "HTTP/1.1"

    def do_POST(self) -> None:
        if self.path not in ("/", "/mcp"):
            self.send_error(404, "use /mcp")
            return
        length = int(self.headers.get("Content-Length", "0"))
        try:
            request = json.loads(self.rfile.read(length))
        except (json.JSONDecodeError, UnicodeDecodeError):
            self._send_json(400, Fixture._error(None, -32700, "parse error"))
            return
        # 2025-06-18 clients normally send this header after initialize.
        protocol = self.headers.get("MCP-Protocol-Version")
        if protocol and protocol not in ("2025-03-26", PROTOCOL_VERSION):
            self._send_json(400, Fixture._error(request.get("id"), -32600, "unsupported MCP-Protocol-Version"))
            return
        response = self.server.fixture.dispatch(request)  # type: ignore[attr-defined]
        if response is None:
            self.send_response(202)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        self._send_json(200, response)

    def _send_json(self, status: int, payload: dict[str, Any]) -> None:
        body = json.dumps(payload, separators=(",", ":")).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.send_header("MCP-Protocol-Version", PROTOCOL_VERSION)
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, _format: str, *_args: Any) -> None:
        return


def serve_http(fixture: Fixture, host: str, port: int) -> None:
    server = ThreadingHTTPServer((host, port), Handler)
    server.fixture = fixture  # type: ignore[attr-defined]
    print(f"MCP fixture listening on http://{host}:{server.server_port}/mcp", file=sys.stderr, flush=True)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass
    finally:
        server.server_close()


def serve_stdio(fixture: Fixture) -> None:
    for line in sys.stdin:
        try:
            request = json.loads(line)
            response = fixture.dispatch(request)
        except (json.JSONDecodeError, UnicodeDecodeError):
            response = Fixture._error(None, -32700, "parse error")
        if response is not None:
            sys.stdout.write(json.dumps(response, separators=(",", ":")) + "\n")
            sys.stdout.flush()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--transport", choices=("http", "stdio"), default="http")
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=8787)
    parser.add_argument("--tools", type=int, default=600)
    parser.add_argument("--page-size", type=int, default=100)
    parser.add_argument("--state-file")
    parser.add_argument("--audit", help="append JSONL call/list audit records to this path")
    args = parser.parse_args()
    try:
        fixture = Fixture(args.tools, args.page_size, args.state_file, args.audit)
    except ValueError as exc:
        parser.error(str(exc))
    if args.transport == "http":
        serve_http(fixture, args.host, args.port)
    else:
        serve_stdio(fixture)


if __name__ == "__main__":
    main()
