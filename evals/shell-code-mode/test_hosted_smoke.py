import json
import unittest
from hosted_smoke import grade


class EvidenceTests(unittest.TestCase):
    def events(self):
        artifact = {"sum": 650, "count": 12, "marker": "test"}
        return [
            {"type": "tool.started", "data": {"tool_call": {"id": "p", "name": "python"}}},
            {"type": "tool.nested_call", "data": {"parent_tool_call_id": "p", "tool_name": "stat_file", "status": "completed"}},
            {"type": "tool.completed", "data": {"tool_name": "write_file", "success": True}},
            {"type": "tool.completed", "data": {"tool_name": "read_file", "success": True, "result": [{"type": "text", "text": json.dumps(artifact)}]}},
            {"type": "turn.completed", "data": {}},
        ]

    def test_requires_artifact_and_authoritative_effects(self):
        events = self.events()
        self.assertTrue(grade(events, "monty", "test")["passed"])
        events[1]["data"]["parent_tool_call_id"] = "other"
        self.assertFalse(grade(events, "monty", "test")["passed"])

    def test_numbered_file_output(self):
        events = self.events()
        artifact = {"sum": 650, "count": 12, "marker": "test"}
        events[3]["data"]["result"] = [{"text": json.dumps({"content": "1|" + json.dumps(artifact), "lines_shown": {"start": 1, "end": 1}, "truncated": False})}]
        self.assertTrue(grade(events, "monty", "test")["passed"])
        events[3]["data"]["result"][0]["text"] = "The file contains " + json.dumps(artifact)
        self.assertFalse(grade(events, "monty", "test")["passed"])

    def test_model_claims_and_repeated_writes_cannot_pass(self):
        events = self.events()
        events[3]["type"] = "message.completed"
        self.assertFalse(grade(events, "monty", "test")["passed"])
        events = self.events()
        events.append(events[2])
        self.assertFalse(grade(events, "monty", "test")["passed"])
        events = self.events()
        events.append({"type": "tool.started", "data": {"tool_call": {"id": "x", "name": "stat_file"}}})
        self.assertFalse(grade(events, "monty", "test")["passed"])


if __name__ == "__main__":
    unittest.main()
