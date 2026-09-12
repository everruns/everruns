import asyncio
import io
import json
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest.mock import patch

from steering import Journal, Owner, reconcile, run_owner, usage_totals
from websockets.exceptions import WebSocketException


def response(
    rid, parent=None, status="in_progress", output=None, tokens=0, reason=None
):
    value = {
        "id": rid,
        "previous_response_id": parent,
        "status": status,
        "output": output or [],
        "usage": {
            "input_tokens": tokens,
            "output_tokens": tokens,
            "input_tokens_details": {"cached_tokens": tokens // 2},
        },
    }
    if reason:
        value["incomplete_details"] = {"reason": reason}
    return value


def created(rid, parent=None):
    return {"type": "response.created", "response": response(rid, parent)}


def terminal(rid, parent=None, status="completed", **kwargs):
    return {
        "type": "response." + status,
        "response": response(rid, parent, status, **kwargs),
    }


def steer(kind, parent="r1", sid="s1", **kwargs):
    return {
        "type": "response.steer." + kind,
        "steer": {"id": sid, "previous_response_id": parent},
        **kwargs,
    }


class StateTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.db = str(Path(self.temp.name) / "session.db")
        self.journal = Journal.create(
            self.db, "Draft a plan", {"instructions": "Be concise"}
        )
        self.owner = Owner(self.journal)
        self.owner.next_request()
        self.owner.event(created("r1"))

    def tearDown(self):
        self.journal.db.close()
        self.temp.cleanup()

    def send_update(self, text="Two weeks only", local_id="u1"):
        self.journal.enqueue(local_id, text)
        request = self.owner.next_request()
        self.assertEqual(
            request,
            {"type": "response.steer", "previous_response_id": "r1", "input": text},
        )
        return request

    def test_acceptance_is_not_application_and_usage_is_per_response(self):
        self.send_update()
        self.owner.event(steer("accepted"))
        self.assertEqual(self.owner.state["intents"][0]["status"], "accepted")
        end = terminal("r1", status="incomplete", reason="steered", tokens=10)
        self.owner.event(end)
        self.assertIsNone(self.owner.next_request())
        self.owner.event(created("r2", "r1"))
        self.assertEqual(self.owner.state["intents"][0]["status"], "applied")
        self.owner.event(terminal("r2", "r1", tokens=20))
        self.owner.event(end)
        self.owner.event(terminal("r2", "r1", tokens=20))
        self.assertEqual(self.owner.state["phase"], "complete")
        self.assertEqual(
            usage_totals(self.owner.state),
            {
                "input_tokens": 30,
                "output_tokens": 30,
                "input_tokens_details": {"cached_tokens": 15},
            },
        )

    def test_completed_parent_waits_for_automatic_successor_and_next_update(self):
        self.send_update()
        self.journal.enqueue("u2", "Use bullets")
        self.owner.event(terminal("r1"))  # Terminal may race the acknowledgement.
        self.assertIsNone(self.owner.next_request())
        self.owner.event(steer("accepted"))
        self.assertIsNone(self.owner.next_request())
        self.owner.event(created("r2", "r1"))
        self.assertEqual(self.owner.next_request()["previous_response_id"], "r2")
        self.owner.event(created("r2", "r1"))
        self.assertEqual(self.owner.state["intents"][1]["status"], "sending")

    def test_pending_tools_and_approval_use_saved_results_once(self):
        self.send_update()
        self.owner.event(steer("accepted"))
        calls = [
            {
                "type": "function_call",
                "call_id": "c1",
                "name": "lookup",
                "arguments": "{}",
            },
            {"type": "mcp_approval_request", "id": "a1", "name": "read"},
        ]
        self.owner.event(terminal("r1", output=calls))
        required = [
            {"type": "function_call_output", "call_id": "c1", "name": "lookup"},
            {"type": "mcp_approval_response", "approval_request_id": "a1"},
        ]
        self.owner.event(
            steer(
                "pending", reason="waiting_for_required_input", required_input=required
            )
        )
        self.assertIsNone(self.owner.next_request())
        result = {
            "type": "function_call_output",
            "call_id": "c1",
            "output": "Saved result",
        }
        self.journal.submit_result(result)
        self.assertIsNone(self.owner.next_request())
        approval = {
            "type": "mcp_approval_response",
            "approval_request_id": "a1",
            "approve": False,
        }
        self.journal.submit_result(approval)
        self.journal.submit_result(result)
        with self.assertRaises(ValueError):
            self.journal.submit_result({**result, "output": "Rerun result"})
        request = self.owner.next_request()
        self.assertEqual(request["input"], [result, approval])
        self.assertNotIn("Two weeks only", json.dumps(request))
        self.assertEqual(request["instructions"], "Be concise")
        self.assertIsNone(self.owner.next_request())
        self.owner.event(created("r2", "r1"))
        self.journal.submit_result(result)  # Producer retry after successor commitment.
        self.assertEqual(self.owner.state["intents"][0]["status"], "applied")

    def test_failures_before_and_after_acceptance_are_preserved(self):
        self.send_update()
        failed = steer("failed", error={"code": "invalid_input"})
        failed["steer"].pop("id")
        failed["steer"]["input"] = "Two weeks only"
        self.owner.event(failed)
        self.assertEqual(self.owner.state["intents"][0]["status"], "failed")
        self.journal.enqueue("u2", "Use bullets")
        self.owner.next_request()
        self.owner.event(steer("accepted", sid="s2"))
        failed = steer("failed", sid="s2", error={"code": "successor_creation_failed"})
        failed["steer"]["input"] = "Use bullets"
        self.owner.event(failed)
        self.assertEqual(
            self.owner.state["intents"][1]["error"]["code"], "successor_creation_failed"
        )
        self.assertIsNone(self.owner.next_request())

    def test_nonsteered_incomplete_is_failure_and_keeps_usage(self):
        self.owner.event(
            terminal("r1", status="incomplete", reason="max_output_tokens", tokens=11)
        )
        self.assertEqual(self.owner.state["phase"], "failed")
        self.assertEqual(usage_totals(self.owner.state)["output_tokens"], 11)

    def test_send_and_accept_crash_windows_never_replay(self):
        for acknowledge in [False, True]:
            with self.subTest(acknowledge=acknowledge):
                if not acknowledge:
                    self.send_update()
                else:
                    self.owner.state["intents"][0]["status"] = "sending"
                    self.owner.event(steer("accepted"))
                self.owner.disconnected("network lost")
                recovered = Owner(self.journal)
                with patch("steering.get_json") as get:
                    self.assertFalse(reconcile(recovered, "test"))
                    get.assert_not_called()
                self.assertIsNone(recovered.next_request())
                self.assertEqual(
                    recovered.state["intents"][0]["text"], "Two weeks only"
                )
                self.assertEqual(recovered.state["intents"][0]["status"], "uncertain")

    def test_reconcile_verified_successor_then_resume_without_duplicate_input(self):
        self.send_update()
        self.owner.event(steer("accepted"))
        self.owner.disconnected("lost successor event")
        child = response("r2", "r1", "completed", tokens=20)
        child["metadata"] = {"everruns_steering_session": self.owner.state["session"]}
        parent = response("r1", status="incomplete", tokens=10, reason="steered")
        with (
            patch("steering.get_json", side_effect=[child, parent]),
            patch(
                "steering.input_items",
                side_effect=[
                    [{"role": "user", "content": "Draft a plan"}],
                    [
                        {"role": "user", "content": "Draft a plan"},
                        {"role": "user", "content": "Two weeks only"},
                    ],
                ],
            ),
        ):
            self.assertTrue(reconcile(self.owner, "test", "r2"))
        self.assertEqual(self.owner.state["intents"][0]["status"], "applied")
        self.assertEqual(usage_totals(self.owner.state)["input_tokens"], 30)
        self.assertIsNone(self.owner.next_request())
        self.journal.enqueue("u2", "Continue")
        next_request = self.owner.next_request()
        self.assertEqual(next_request["type"], "response.create")
        self.assertEqual(
            next_request["input"], [{"role": "user", "content": "Continue"}]
        )

    def test_reconcile_rejects_missing_or_duplicate_input_and_wrong_lineage(self):
        self.send_update()
        self.owner.disconnected("lost acknowledgement")
        child = response("r2", "r1", "completed")
        child["metadata"] = {"everruns_steering_session": self.owner.state["session"]}
        for texts in [[], ["Two weeks only", "Two weeks only"]]:
            with (
                patch("steering.get_json", return_value=child),
                patch(
                    "steering.input_items",
                    side_effect=[
                        [],
                        [{"role": "user", "content": text} for text in texts],
                    ],
                ),
                self.assertRaises(ValueError),
            ):
                reconcile(self.owner, "test", "r2")
        with (
            patch(
                "steering.get_json",
                return_value={**child, "previous_response_id": "foreign"},
            ),
            self.assertRaises(ValueError),
        ):
            reconcile(self.owner, "test", "r2")
        self.assertEqual(self.owner.state["intents"][0]["status"], "uncertain")

    def test_parent_failure_drains_steering_failure_before_stopping(self):
        self.send_update()
        self.owner.event(steer("accepted"))
        self.owner.event(terminal("r1", status="failed", tokens=4))
        self.assertEqual(self.owner.state["phase"], "waiting")
        failure = steer("failed", error={"code": "successor_creation_failed"})
        failure["steer"]["input"] = "Two weeks only"
        self.owner.event(failure)
        self.assertEqual(self.owner.state["phase"], "failed")
        self.assertEqual(self.owner.state["intents"][0]["status"], "failed")
        self.assertEqual(usage_totals(self.owner.state)["output_tokens"], 4)

    def test_quota_error_and_uncommitted_input_survive_disconnect(self):
        self.send_update()
        self.owner.event(steer("accepted"))
        error = {"code": "credit_balance_exhausted", "message": "No credits remaining"}
        self.owner.event({"type": "error", "error": error})
        self.owner.disconnected("Owner stopped")
        self.assertEqual(self.journal.load()["error"], error)
        self.assertEqual(self.journal.load()["intents"][0]["status"], "uncertain")

    def test_lost_initial_create_receipt_can_be_reconciled(self):
        self.owner.state = self.journal.load()
        self.owner.state.update(current=None, responses={}, phase="new", create=None)
        self.owner.next_request()
        self.owner.disconnected("lost initial response id")
        child = response("r1", status="completed", tokens=3)
        child["metadata"] = {"everruns_steering_session": self.owner.state["session"]}
        with (
            patch("steering.get_json", return_value=child),
            patch(
                "steering.input_items",
                return_value=[{"role": "user", "content": "Draft a plan"}],
            ),
        ):
            self.assertTrue(reconcile(self.owner, "test", "r1"))
        self.assertIsNone(self.owner.next_request())
        self.assertEqual(self.owner.state["phase"], "complete")

    def test_recovery_of_known_successor_waiting_for_tools(self):
        self.send_update()
        self.owner.event(steer("accepted"))
        self.owner.event(
            terminal("r1", status="incomplete", reason="steered", tokens=2)
        )
        self.owner.event(created("r2", "r1"))
        self.owner.disconnected("lost terminal event")
        child = response(
            "r2",
            "r1",
            "completed",
            tokens=3,
            output=[
                {
                    "type": "function_call",
                    "call_id": "c1",
                    "name": "lookup",
                    "arguments": "{}",
                }
            ],
        )
        with patch("steering.get_json", return_value=child):
            self.assertTrue(reconcile(self.owner, "test"))
        self.assertEqual(self.owner.state["phase"], "waiting")
        output = {
            "type": "function_call_output",
            "call_id": "c1",
            "output": "Already ran",
        }
        self.journal.submit_result(output)
        request = self.owner.next_request()
        self.assertEqual(request["input"], [output])
        self.assertEqual(request["previous_response_id"], "r2")
        self.assertEqual(usage_totals(self.owner.state)["output_tokens"], 5)

    def test_recovery_of_tool_create_requires_saved_result_evidence(self):
        self.owner.event(
            terminal(
                "r1",
                output=[
                    {
                        "type": "function_call",
                        "call_id": "c1",
                        "name": "lookup",
                        "arguments": "{}",
                    }
                ],
            )
        )
        output = {
            "type": "function_call_output",
            "call_id": "c1",
            "output": "Already ran",
        }
        self.journal.submit_result(output)
        self.owner.next_request()
        self.owner.disconnected("lost tool continuation receipt")
        child = response("r2", "r1", "completed")
        child["metadata"] = {"everruns_steering_session": self.owner.state["session"]}
        with (
            patch("steering.get_json", return_value=child),
            patch("steering.input_items", side_effect=[[], []]),
            self.assertRaises(ValueError),
        ):
            reconcile(self.owner, "test", "r2")
        with (
            patch("steering.get_json", return_value=child),
            patch("steering.input_items", side_effect=[[], [output]]),
        ):
            self.assertTrue(reconcile(self.owner, "test", "r2"))
        self.assertEqual(self.owner.state["phase"], "complete")

    def test_required_approval_is_never_inferred(self):
        self.owner.event(
            terminal("r1", output=[{"type": "mcp_approval_request", "id": "a1"}])
        )
        with self.assertRaises(ValueError):
            self.journal.submit_result(
                {"type": "mcp_approval_response", "approval_request_id": "a1"}
            )
        with self.assertRaises(ValueError):
            self.journal.submit_result(
                {
                    "type": "mcp_approval_response",
                    "approval_request_id": "foreign",
                    "approve": True,
                }
            )
        self.assertIsNone(self.owner.next_request())

    def test_producer_retries_and_second_owner_are_fenced(self):
        producer = Journal(self.db)
        try:
            producer.enqueue("u1", "hello")
            producer.enqueue("u1", "hello")
            with self.assertRaises(ValueError):
                producer.enqueue("u1", "changed")
            self.owner.next_request()
            self.assertEqual(len(self.owner.state["intents"]), 1)
            with (
                self.journal.ownership(),
                self.assertRaises(BlockingIOError),
                producer.ownership(),
            ):
                self.fail("Two owners acquired the same journal")
        finally:
            producer.db.close()


class WireTests(unittest.IsolatedAsyncioTestCase):
    async def test_automatic_successor_on_original_socket_for_both_parent_endings(self):
        from websockets.asyncio.server import serve

        for status in ["completed", "incomplete"]:
            with self.subTest(status=status), tempfile.TemporaryDirectory() as tmp:
                journal = Journal.create(str(Path(tmp) / "session.db"), "Plan", {})
                journal.enqueue("u1", "Two weeks only")
                errors = []

                async def server(socket, ending=status, failures=errors):
                    try:
                        self.assertEqual(
                            json.loads(await socket.recv())["type"], "response.create"
                        )
                        await socket.send(json.dumps(created("r1")))
                        self.assertEqual(
                            json.loads(await socket.recv())["type"], "response.steer"
                        )
                        await socket.send(json.dumps(steer("accepted")))
                        await socket.send(
                            json.dumps(
                                terminal(
                                    "r1",
                                    status=ending,
                                    reason="steered"
                                    if ending == "incomplete"
                                    else None,
                                    tokens=4,
                                )
                            )
                        )
                        with self.assertRaises(TimeoutError):
                            await asyncio.wait_for(socket.recv(), 0.05)
                        await socket.send(json.dumps(created("r2", "r1")))
                        await socket.send(json.dumps(terminal("r2", "r1", tokens=5)))
                        await socket.wait_closed()
                    except (
                        AssertionError,
                        OSError,
                        ValueError,
                        WebSocketException,
                    ) as error:
                        failures.append(error)

                try:
                    async with serve(server, "127.0.0.1", 0) as listener:
                        port = listener.sockets[0].getsockname()[1]
                        with redirect_stdout(io.StringIO()):
                            await run_owner(
                                journal, "test", f"ws://127.0.0.1:{port}", True, 5
                            )
                    if errors:
                        raise errors[0]
                    self.assertEqual(journal.load()["phase"], "complete")
                    self.assertEqual(journal.load()["intents"][0]["status"], "applied")
                    self.assertEqual(usage_totals(journal.load())["output_tokens"], 9)
                finally:
                    journal.db.close()

    async def test_real_websocket_and_independent_producer_tool_continuation(self):
        from websockets.asyncio.server import serve

        with tempfile.TemporaryDirectory() as tmp:
            path = str(Path(tmp) / "journal.db")
            journal = Journal.create(path, "Draft a plan", {})
            producer = Journal(path)
            received = []
            errors = []

            async def server(socket):
                try:
                    received.append(json.loads(await socket.recv()))
                    self.assertEqual(received[-1]["type"], "response.create")
                    await socket.send(json.dumps(created("r1")))
                    process = await asyncio.create_subprocess_exec(
                        sys.executable,
                        str(Path(__file__).with_name("steering.py")),
                        "--db",
                        path,
                        "update",
                        "Two weeks only",
                        "--id",
                        "human-update-1",
                    )
                    self.assertEqual(await process.wait(), 0)
                    received.append(json.loads(await socket.recv()))
                    self.assertEqual(
                        received[-1],
                        {
                            "type": "response.steer",
                            "previous_response_id": "r1",
                            "input": "Two weeks only",
                        },
                    )
                    await socket.send(json.dumps(steer("accepted")))
                    await socket.send(
                        json.dumps(
                            terminal(
                                "r1",
                                tokens=7,
                                output=[
                                    {
                                        "type": "function_call",
                                        "call_id": "c1",
                                        "name": "lookup",
                                        "arguments": "{}",
                                    }
                                ],
                            )
                        )
                    )
                    await socket.send(
                        json.dumps(
                            steer(
                                "pending",
                                reason="waiting_for_required_input",
                                required_input=[
                                    {
                                        "type": "function_call_output",
                                        "call_id": "c1",
                                        "name": "lookup",
                                    }
                                ],
                            )
                        )
                    )
                    for _ in range(100):
                        if producer.load()["required"]:
                            break
                        await asyncio.sleep(0.01)
                    producer.submit_result(
                        {
                            "type": "function_call_output",
                            "call_id": "c1",
                            "output": "Design ready",
                        }
                    )
                    received.append(json.loads(await socket.recv()))
                    self.assertEqual(
                        received[-1]["input"],
                        [
                            {
                                "type": "function_call_output",
                                "call_id": "c1",
                                "output": "Design ready",
                            }
                        ],
                    )
                    await socket.send(json.dumps(created("r2", "r1")))
                    await socket.send(
                        json.dumps(
                            terminal(
                                "r2",
                                "r1",
                                tokens=9,
                                output=[
                                    {
                                        "type": "message",
                                        "role": "assistant",
                                        "content": [
                                            {
                                                "type": "output_text",
                                                "text": "Two-week plan",
                                            }
                                        ],
                                    }
                                ],
                            )
                        )
                    )
                    await socket.wait_closed()
                except (
                    AssertionError,
                    OSError,
                    ValueError,
                    WebSocketException,
                ) as error:
                    errors.append(error)

            try:
                async with serve(server, "127.0.0.1", 0) as listener:
                    port = listener.sockets[0].getsockname()[1]
                    with redirect_stdout(io.StringIO()):
                        await run_owner(
                            journal, "test-key", f"ws://127.0.0.1:{port}", True, 5
                        )
                if errors:
                    raise errors[0]
                self.assertEqual(len(received), 3)
                state = journal.load()
                self.assertEqual(state["phase"], "complete")
                self.assertEqual(state["intents"][0]["status"], "applied")
                self.assertEqual(usage_totals(state)["input_tokens"], 16)
                self.assertEqual(
                    state["responses"]["r2"]["output"][0]["content"][0]["text"],
                    "Two-week plan",
                )
            finally:
                producer.db.close()
                journal.db.close()


if __name__ == "__main__":
    unittest.main()
