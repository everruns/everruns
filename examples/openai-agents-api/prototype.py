#!/usr/bin/env python3
"""Opt-in OpenAI Agents API protocol prototype."""

import argparse
import json
import os
import sys
import uuid
from datetime import datetime, timezone
from urllib.parse import quote
from urllib.request import Request, urlopen

FEATURE_FLAG = "EVERRUNS_OPENAI_AGENTS_API"
API_BASE = "https://api.openai.com/v1"


def build_session_request(runtime_agent, prompt, mcp_server):
    tools = []
    for tool in runtime_agent.get("tools", []):
        if tool.get("type") != "client_side":
            raise ValueError("The prototype only proxies client-side function tools")
        tools.append(
            {
                "type": "function",
                "name": tool["name"],
                "description": tool["description"],
                "parameters": tool["parameters"],
            }
        )
    tools.append(
        {
            "type": "mcp",
            "server_label": mcp_server["server_label"],
            "transport": {
                "type": "http",
                "server_url": mcp_server["server_url"],
            },
            "connection_origin": "service",
            "allowed_tools": [mcp_server["tool_name"]],
            "required": True,
        }
    )
    return {
        "agent": {
            "model": runtime_agent["model"],
            "instructions": runtime_agent["system_prompt"],
            "tools": tools,
            "multi_agent": {"enabled": False},
        },
        "environment": {"type": "none"},
        "input": [
            {
                "role": "user",
                "content": [{"type": "input_text", "text": prompt}],
            }
        ],
        "stream": True,
    }


def parse_sse(response):
    try:
        data = []
        for raw_line in response:
            line = raw_line.decode("utf-8").rstrip("\r\n")
            if not line:
                if data:
                    yield json.loads("\n".join(data))
                    data = []
                continue
            if line.startswith("data:"):
                value = line[5:].lstrip()
                if value != "[DONE]":
                    data.append(value)
        if data:
            yield json.loads("\n".join(data))
    finally:
        response.close()


class OpenAiAgentsHttp:
    def __init__(self, api_key, base=API_BASE, opener=urlopen):
        self.api_key = api_key
        self.base = base.rstrip("/")
        self.opener = opener

    def _request(self, method, path, body=None, accept="application/json"):
        headers = {
            "Authorization": "Bearer " + self.api_key,
            "OpenAI-Beta": "agents=v1",
            "Accept": accept,
        }
        data = None
        if body is not None:
            headers["Content-Type"] = "application/json"
            data = json.dumps(body).encode()
        return self.opener(
            Request(self.base + path, data=data, headers=headers, method=method),
            timeout=30,
        )

    def create_session(self, body):
        return parse_sse(
            self._request("POST", "/agents/sessions", body, "text/event-stream")
        )

    def stream_events(self, session_id):
        path = "/agents/sessions/" + quote(session_id, safe="") + "/events?stream=true"
        return parse_sse(self._request("GET", path, accept="text/event-stream"))

    def retrieve_session(self, session_id):
        path = "/agents/sessions/" + quote(session_id, safe="")
        with self._request("GET", path) as response:
            return json.load(response)

    def send_events(self, session_id, events):
        path = "/agents/sessions/" + quote(session_id, safe="") + "/events"
        with self._request("POST", path, {"events": events}) as response:
            return json.load(response)


class EverrunsEventMapper:
    def __init__(
        self,
        session_id,
        turn_id,
        input_message_id,
        output_message_id,
        model,
        now=None,
    ):
        self.session_id = session_id
        self.turn_id = turn_id
        self.input_message_id = input_message_id
        self.output_message_id = output_message_id
        self.model = model
        self.now = now or (lambda: datetime.now(timezone.utc).isoformat())
        self.text = ""
        self.started = False
        self.terminal = False
        self.tool_calls = {}

    def _event(self, event_type, data):
        return {
            "type": event_type,
            "ts": self.now(),
            "session_id": self.session_id,
            "context": {
                "turn_id": self.turn_id,
                "input_message_id": self.input_message_id,
            },
            "data": data,
            "metadata": {"provider": "openai_agents_api"},
        }

    def _usage(self, turn):
        usage = turn.get("usage") or {}
        if not usage:
            return None
        return {
            "input_tokens": usage.get("input_tokens", 0),
            "output_tokens": usage.get("output_tokens", 0),
        }

    def map(self, event):
        kind = event.get("type")
        turn = event.get("turn") or {}
        if turn.get("subagent_id") is not None:
            return []
        if kind == "agent.session.turn.in_progress":
            return [
                self._event(
                    "session.activated",
                    {
                        "turn_id": self.turn_id,
                        "input_message_id": self.input_message_id,
                    },
                ),
                self._event(
                    "turn.started",
                    {
                        "turn_id": self.turn_id,
                        "input_message_id": self.input_message_id,
                        "input_content": None,
                        "agent_id": None,
                        "agent_name": None,
                        "agent_description": None,
                    },
                ),
            ]
        if kind == "agent.session.turn.output_text.delta":
            delta = event.get("delta", "")
            if not isinstance(delta, str):
                raise ValueError("output_text.delta must contain a string delta")
            mapped = []
            if not self.started:
                mapped.append(
                    self._event(
                        "output.message.started",
                        {
                            "turn_id": self.turn_id,
                            "message_id": self.output_message_id,
                            "model": self.model,
                            "iteration": 1,
                        },
                    )
                )
                self.started = True
            self.text += delta
            mapped.append(
                self._event(
                    "output.message.delta",
                    {
                        "turn_id": self.turn_id,
                        "message_id": self.output_message_id,
                        "delta": delta,
                        "accumulated": self.text,
                    },
                )
            )
            return mapped
        if kind == "agent.session.turn.item.added":
            item = event.get("item") or {}
            if item.get("type") not in {"function_call", "mcp_tool_call"}:
                return []
            call_id = item.get("call_id") or item.get("id")
            name = item.get("name") or item.get("server_label") or item["type"]
            arguments = item.get("arguments") or {}
            self.tool_calls[call_id] = name
            return [
                self._event(
                    "tool.started",
                    {
                        "tool_call": {
                            "id": call_id,
                            "name": name,
                            "arguments": arguments,
                        }
                    },
                )
            ]
        if kind == "agent.session.turn.completed":
            self.terminal = True
            usage = self._usage(turn)
            completed = []
            if self.started:
                completed.append(
                    self._event(
                        "output.message.completed",
                        {
                            "message": {
                                "id": self.output_message_id,
                                "role": "agent",
                                "content": [{"type": "text", "text": self.text}],
                                "created_at": self.now(),
                            },
                            "metadata": {"model": self.model},
                            "usage": usage,
                        },
                    )
                )
            completed.extend(
                [
                    self._event(
                        "turn.completed",
                        {
                            "turn_id": self.turn_id,
                            "iterations": 1,
                            "usage": usage,
                            "final_message_id": self.output_message_id
                            if self.started
                            else None,
                            "final_answer_preview": self.text[:200] or None,
                            "status": "completed",
                        },
                    ),
                    self._event(
                        "session.idled",
                        {
                            "turn_id": self.turn_id,
                            "iterations": 1,
                            "usage": usage,
                        },
                    ),
                ]
            )
            return completed
        if kind == "agent.session.turn.failed":
            self.terminal = True
            error = turn.get("error") or event.get("error") or {}
            message = error.get("message") if isinstance(error, dict) else str(error)
            return [
                self._event(
                    "turn.failed",
                    {
                        "turn_id": self.turn_id,
                        "error": message or "OpenAI Agents API turn failed",
                        "error_code": error.get("code")
                        if isinstance(error, dict)
                        else None,
                    },
                )
            ]
        if kind == "agent.session.turn.cancelled":
            self.terminal = True
            return [
                self._event(
                    "turn.cancelled",
                    {
                        "turn_id": self.turn_id,
                        "reason": turn.get("reason"),
                        "usage": self._usage(turn),
                    },
                )
            ]
        if kind in {
            "agent.session.failed",
            "agent.session.environment.failed",
            "error",
        }:
            self.terminal = True
            error = event.get("error") or {}
            message = error.get("message") if isinstance(error, dict) else str(error)
            return [
                self._event(
                    "turn.failed",
                    {
                        "turn_id": self.turn_id,
                        "error": message or kind,
                        "error_code": error.get("code")
                        if isinstance(error, dict)
                        else kind,
                    },
                )
            ]
        return []

    def requested_tools(self, actions):
        calls = [
            {
                "id": action["call_id"],
                "name": action["name"],
                "arguments": action.get("arguments", {}),
            }
            for action in actions
            if action.get("type") == "function_call"
        ]
        if not calls:
            return []
        return [
            self._event(
                "tool.call_requested",
                {
                    "tool_calls": calls,
                    "tool_summaries": [],
                    "headline": None,
                    "completed_headline": None,
                },
            )
        ]

    def completed_tool(self, action, output=None, error=None):
        success = error is None
        return self._event(
            "tool.completed",
            {
                "tool_call_id": action["call_id"],
                "tool_name": action["name"],
                "success": success,
                "status": "success" if success else "error",
                "result": [{"type": "text", "text": output}] if success else None,
                "error": error,
            },
        )


class Prototype:
    def __init__(self, transport, emit, functions):
        self.transport = transport
        self.emit = emit
        self.functions = functions

    def _handle_actions(self, session_id, mapper, actions):
        self.emit.extend(mapper.requested_tools(actions))
        results = []
        for action in actions:
            if action.get("type") != "function_call":
                raise RuntimeError(
                    "Unsupported required action: " + str(action.get("type"))
                )
            handler = self.functions.get(action["name"])
            if handler is None:
                raise RuntimeError("No handler for function " + action["name"])
            try:
                output = json.dumps(handler(action.get("arguments", {})))
                result = {
                    "type": "agent.session.input.tool_result",
                    "turn_id": action["turn_id"],
                    "call_id": action["call_id"],
                    "success": True,
                    "output": output,
                }
                self.emit.append(mapper.completed_tool(action, output=output))
            except Exception as error:
                result = {
                    "type": "agent.session.input.tool_result",
                    "turn_id": action["turn_id"],
                    "call_id": action["call_id"],
                    "success": False,
                    "error": str(error),
                }
                self.emit.append(mapper.completed_tool(action, error=str(error)))
            results.append(result)
        self.transport.send_events(session_id, results)

    def _consume(self, stream, mapper, session_id=None):
        for event in stream:
            session = event.get("session") or {}
            session_id = session.get("id") or event.get("session_id") or session_id
            if event.get("type") == "agent.session.requires_action":
                actions = session.get("required_actions") or []
                if not actions:
                    if session_id is None:
                        raise RuntimeError("Required action event has no session ID")
                    actions = self.transport.retrieve_session(session_id).get(
                        "required_actions", []
                    )
                self._handle_actions(session_id, mapper, actions)
            else:
                self.emit.extend(mapper.map(event))
        if not mapper.terminal:
            raise RuntimeError(
                "Agents event stream closed before a root turn ended; "
                "retrieve saved state"
            )
        return session_id

    def run(self, request, mapper):
        if os.environ.get(FEATURE_FLAG) != "1":
            raise RuntimeError(FEATURE_FLAG + "=1 is required")
        return self._consume(self.transport.create_session(request), mapper)

    def recover(self, session_id, mapper):
        if os.environ.get(FEATURE_FLAG) != "1":
            raise RuntimeError(FEATURE_FLAG + "=1 is required")
        session = self.transport.retrieve_session(session_id)
        actions = session.get("required_actions") or []
        if actions:
            self._handle_actions(session_id, mapper, actions)
        return self._consume(
            self.transport.stream_events(session_id), mapper, session_id
        )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--agent", required=True, help="RuntimeAgent JSON")
    parser.add_argument("--mcp", required=True, help="MCP server JSON")
    parser.add_argument("prompt")
    args = parser.parse_args()
    with open(args.agent, encoding="utf-8") as file:
        runtime_agent = json.load(file)
    with open(args.mcp, encoding="utf-8") as file:
        mcp_server = json.load(file)
    ids = {
        name: name + "_" + uuid.uuid4().hex
        for name in ("session", "turn", "message")
    }
    mapper = EverrunsEventMapper(
        ids["session"],
        ids["turn"],
        ids["message"],
        "message_" + uuid.uuid4().hex,
        runtime_agent["model"],
    )
    events = []
    prototype = Prototype(
        OpenAiAgentsHttp(os.environ["OPENAI_API_KEY"]),
        events,
        {
            "get_customer": lambda arguments: {
                "id": arguments["id"],
                "found": arguments["id"] == "123",
            }
        },
    )
    prototype.run(
        build_session_request(runtime_agent, args.prompt, mcp_server),
        mapper,
    )
    print(json.dumps(events, indent=2))


if __name__ == "__main__":
    try:
        main()
    except (KeyError, RuntimeError, ValueError) as error:
        sys.exit(str(error))
