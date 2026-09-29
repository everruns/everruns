import io
import json
import os
import unittest
from unittest.mock import patch

from prototype import (
    EverrunsEventMapper,
    FEATURE_FLAG,
    OpenAiAgentsHttp,
    Prototype,
    build_session_request,
    parse_sse,
)


class FakeTransport:
    def __init__(self, events, session=None):
        self.events = events
        self.session = session or {}
        self.created = []
        self.sent = []

    def create_session(self, request):
        self.created.append(request)
        return iter(self.events)

    def retrieve_session(self, session_id):
        return self.session

    def send_events(self, session_id, events):
        self.sent.append((session_id, events))
        return {}

    def stream_events(self, session_id):
        return iter(self.events)


def mapper():
    return EverrunsEventMapper(
        "session_00000000000000000000000000000001",
        "turn_00000000000000000000000000000002",
        "message_00000000000000000000000000000003",
        "message_00000000000000000000000000000004",
        "gpt-6-astra",
        now=lambda: "2026-09-29T00:00:00+00:00",
    )


class PrototypeTests(unittest.TestCase):
    def runtime_agent(self):
        return {
            "model": "gpt-6-astra",
            "system_prompt": "Use tools before answering.",
            "tools": [
                {
                    "type": "client_side",
                    "name": "get_customer",
                    "description": "Look up a customer",
                    "parameters": {
                        "type": "object",
                        "properties": {"id": {"type": "string"}},
                        "required": ["id"],
                        "additionalProperties": False,
                    },
                }
            ],
        }

    def request(self):
        return build_session_request(
            self.runtime_agent(),
            "Find customer 123",
            {
                "server_label": "openai_docs",
                "server_url": "https://developers.openai.com/mcp",
                "tool_name": "search_openai_docs",
            },
        )

    def test_maps_runtime_agent_function_and_mcp_tools(self):
        request = self.request()
        self.assertEqual(request["environment"], {"type": "none"})
        self.assertTrue(request["stream"])
        self.assertEqual(
            request["agent"]["tools"],
            [
                {
                    "type": "function",
                    "name": "get_customer",
                    "description": "Look up a customer",
                    "parameters": self.runtime_agent()["tools"][0]["parameters"],
                },
                {
                    "type": "mcp",
                    "server_label": "openai_docs",
                    "transport": {
                        "type": "http",
                        "server_url": "https://developers.openai.com/mcp",
                    },
                    "connection_origin": "service",
                    "allowed_tools": ["search_openai_docs"],
                    "required": True,
                },
            ],
        )
        with self.assertRaises(ValueError):
            build_session_request(
                {
                    **self.runtime_agent(),
                    "tools": [{"type": "builtin", "name": "unsafe"}],
                },
                "prompt",
                {
                    "server_label": "mcp",
                    "server_url": "https://example.com/mcp",
                    "tool_name": "search",
                },
            )

    def test_http_transport_uses_beta_header_and_documented_endpoints(self):
        requests = []
        responses = [
            io.BytesIO(
                b'data: {"type":"agent.session.created","session_id":"sess_1"}\n\n'
                b"data: [DONE]\n\n"
            ),
            io.BytesIO(b"{}"),
        ]

        def opener(request, timeout):
            requests.append((request, timeout))
            return responses.pop(0)

        transport = OpenAiAgentsHttp(
            "test-key", base="https://example.test/v1", opener=opener
        )
        self.assertEqual(
            list(transport.create_session({"stream": True}))[0]["session_id"],
            "sess_1",
        )
        transport.send_events(
            "sess_1",
            [{"type": "agent.session.input.tool_result", "call_id": "call_1"}],
        )
        self.assertEqual(
            [entry[0].full_url for entry in requests],
            [
                "https://example.test/v1/agents/sessions",
                "https://example.test/v1/agents/sessions/sess_1/events",
            ],
        )
        headers = dict(requests[0][0].header_items())
        self.assertEqual(headers["Openai-beta"], "agents=v1")
        self.assertEqual(headers["Accept"], "text/event-stream")
        self.assertEqual(requests[0][1], 30)

    def test_sse_parser_handles_multiline_data_and_final_event(self):
        response = io.BytesIO(
            b'event: update\ndata: {"type":"first",\ndata: "value":1}\n\n'
            b'data: {"type":"second"}'
        )
        self.assertEqual(
            list(parse_sse(response)),
            [{"type": "first", "value": 1}, {"type": "second"}],
        )

    @patch.dict(os.environ, {FEATURE_FLAG: "1"})
    def test_function_result_and_stream_map_to_canonical_session_events(self):
        action = {
            "type": "function_call",
            "turn_id": "provider_turn_1",
            "call_id": "call_1",
            "name": "get_customer",
            "arguments": {"id": "123"},
        }
        events = [
            {
                "type": "agent.session.turn.in_progress",
                "session_id": "sess_1",
                "turn": {"id": "provider_turn_1", "subagent_id": None},
            },
            {
                "type": "agent.session.turn.item.added",
                "item": {
                    "type": "mcp_tool_call",
                    "id": "mcp_1",
                    "server_label": "openai_docs",
                    "arguments": {"query": "Agents API"},
                },
                "turn": {"subagent_id": None},
            },
            {
                "type": "agent.session.requires_action",
                "session": {"id": "sess_1", "required_actions": [action]},
            },
            {
                "type": "agent.session.turn.output_text.delta",
                "delta": "Customer ",
                "turn": {"subagent_id": None},
            },
            {
                "type": "agent.session.turn.output_text.delta",
                "delta": "found.",
                "turn": {"subagent_id": None},
            },
            {
                "type": "agent.session.turn.completed",
                "turn": {
                    "subagent_id": None,
                    "usage": {
                        "input_tokens": 10,
                        "output_tokens": 2,
                        "total_tokens": 12,
                    },
                },
            },
        ]
        transport = FakeTransport(events)
        emitted = []
        prototype = Prototype(
            transport,
            emitted,
            {"get_customer": lambda arguments: {"id": arguments["id"], "found": True}},
        )
        session_id = prototype.run(self.request(), mapper())
        self.assertEqual(session_id, "sess_1")
        self.assertEqual(transport.created, [self.request()])
        self.assertEqual(
            transport.sent,
            [
                (
                    "sess_1",
                    [
                        {
                            "type": "agent.session.input.tool_result",
                            "turn_id": "provider_turn_1",
                            "call_id": "call_1",
                            "success": True,
                            "output": json.dumps({"id": "123", "found": True}),
                        }
                    ],
                )
            ],
        )
        types = [event["type"] for event in emitted]
        self.assertEqual(
            types,
            [
                "session.activated",
                "turn.started",
                "tool.started",
                "tool.call_requested",
                "tool.completed",
                "output.message.started",
                "output.message.delta",
                "output.message.delta",
                "output.message.completed",
                "turn.completed",
                "session.idled",
            ],
        )
        self.assertEqual(emitted[-5]["data"]["accumulated"], "Customer ")
        self.assertEqual(emitted[-4]["data"]["accumulated"], "Customer found.")
        self.assertEqual(
            emitted[-3]["data"]["message"]["content"][0]["text"], "Customer found."
        )
        self.assertTrue(
            all(
                event["metadata"]["provider"] == "openai_agents_api"
                for event in emitted
            )
        )

    @patch.dict(os.environ, {FEATURE_FLAG: "1"})
    def test_required_actions_can_be_retrieved_after_reconnect(self):
        action = {
            "type": "function_call",
            "turn_id": "provider_turn_1",
            "call_id": "call_1",
            "name": "get_customer",
            "arguments": {"id": "123"},
        }
        transport = FakeTransport(
            [
                {
                    "type": "agent.session.turn.completed",
                    "turn": {"subagent_id": None},
                }
            ],
            session={"required_actions": [action]},
        )
        emitted = []
        prototype = Prototype(
            transport, emitted, {"get_customer": lambda arguments: arguments}
        )
        self.assertEqual(prototype.recover("sess_1", mapper()), "sess_1")
        self.assertEqual(transport.sent[0][0], "sess_1")
        self.assertIn("tool.call_requested", [event["type"] for event in emitted])

    @patch.dict(os.environ, {FEATURE_FLAG: "1"})
    def test_failures_unknown_events_and_incomplete_streams_fail_closed(self):
        mapped = mapper().map({"type": "future.event", "payload": "ignored"})
        self.assertEqual(mapped, [])
        failure_mapper = mapper()
        failed = failure_mapper.map(
            {
                "type": "agent.session.turn.failed",
                "turn": {
                    "subagent_id": None,
                    "error": {"code": "rate_limit", "message": "try later"},
                },
            }
        )
        self.assertEqual(failed[0]["type"], "turn.failed")
        self.assertEqual(failed[0]["data"]["error_code"], "rate_limit")
        with self.assertRaisesRegex(RuntimeError, "closed before"):
            Prototype(
                FakeTransport(
                    [
                        {
                            "type": "agent.session.turn.output_text.delta",
                            "delta": "partial",
                            "turn": {"subagent_id": None},
                        }
                    ]
                ),
                [],
                {},
            ).run(self.request(), mapper())

    def test_feature_is_off_by_default(self):
        with patch.dict(os.environ, {}, clear=True), self.assertRaisesRegex(
            RuntimeError, FEATURE_FLAG
        ):
            Prototype(FakeTransport([]), [], {}).run(self.request(), mapper())


if __name__ == "__main__":
    unittest.main()
