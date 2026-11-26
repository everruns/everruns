import unittest
from hosted_recovery import approval, grade


class RecoveryEvidenceTests(unittest.TestCase):
    def trace(self):
        return [
            {"type": "tool.started", "data": {"tool_call": {"id": "p", "name": "python", "arguments": {"script": "program"}}}},
            {"type": "tool.started", "data": {"tool_call": {"id": "r", "name": "python", "arguments": {"resume": "saved"}}}},
            *[{"type": "tool.nested_call", "data": {"parent_tool_call_id": parent, "tool_name": name, "status": "completed"}} for name, parent in (("sql_execute", "p"), ("delete_file", "r"))],
            {"type": "tool.started", "data": {"tool_call": {"id": "q", "name": "sql_query", "arguments": {"database": "montyproof", "sql": "SELECT COUNT(*) AS receipts FROM receipts"}}}},
            {"type": "tool.completed", "data": {"tool_call_id": "q", "tool_name": "sql_query", "success": True, "result": {"columns": ["receipts"], "rows": [[1]]}}},
        ]

    def test_requires_actual_receipt_and_new_executor(self):
        fault = {"old_executor_pid": 1, "new_executor_pid": 2, "old_executor_exited": True}
        t = self.trace()
        self.assertTrue(grade(t, "allow", False, fault)["passed"])
        t[-1]["data"]["result"]["rows"] = [[2]]
        self.assertFalse(grade(t, "allow", False, fault)["passed"])
        self.assertFalse(grade(self.trace(), "allow", False, {})["passed"])
        self.assertFalse(grade(self.trace(), "allow", False, dict(fault, leader_before=1, leader_after=1))["passed"])
        t = self.trace()
        t[-2]["data"]["tool_call"]["arguments"]["sql"] = "SELECT 1 AS receipts"
        self.assertFalse(grade(t, "allow", False, fault)["passed"])
        self.assertFalse(grade(self.trace(), "allow", False, fault, "another program")["passed"])

    def test_rejection_and_duplicate_source(self):
        fault = {"old_executor_pid": 1, "new_executor_pid": 2, "old_executor_exited": True}
        t = self.trace()
        self.assertFalse(grade(t, "reject", True, fault)["passed"])
        del t[3]
        self.assertTrue(grade(t, "reject", True, fault)["passed"])
        t.append(t[0])
        self.assertFalse(grade(t, "reject", True, fault)["passed"])

    def test_only_answers_declared_approval(self):
        self.assertIsNone(approval([]))
        event = {"type": "tool.call_requested", "data": {"tool_calls": [{"name": "approve_tool_call", "arguments": {"tool": "delete_file", "arguments": {"path": "/workspace/unrelated.txt"}}}]}}
        with self.assertRaises(ValueError):
            approval([event])


if __name__ == "__main__":
    unittest.main()
