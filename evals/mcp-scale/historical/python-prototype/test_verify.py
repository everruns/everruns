import unittest

from fixture.server import Fixture
from verify_rollback import verify


class VerificationTests(unittest.TestCase):
    def test_correct_final_state_without_required_evidence_fails(self):
        fixture = Fixture()
        fixture._execute("rollback_deployment", {"deployment_id": "dep-bill-stg-2042", "reason": "test"})
        self.assertFalse(verify(fixture.state, [])["task_passed"])

    def test_grounded_rollback_and_verification_pass_and_detect_prod_change(self):
        fixture = Fixture()
        sequence = [
            ("list_deployments", {"service": "billing-api", "environment": "staging"}),
            ("search_logs", {"service": "billing-api", "environment": "staging", "query": "500", "time_range": "1h"}),
            ("rollback_deployment", {"deployment_id": "dep-bill-stg-2042", "reason": "test"}),
            ("get_deployment", {"deployment_id": "dep-bill-stg-2042"}),
        ]
        audit = []
        for name, arguments in sequence:
            fixture._execute(name, arguments)
            audit.append({"method": "tools/call", "tool": name, "arguments": arguments, "ok": True})
        self.assertTrue(verify(fixture.state, audit)["task_passed"])
        fixture.state["deployments"]["dep-bill-prd-9911"]["version"] = "wrong"
        self.assertFalse(verify(fixture.state, audit)["task_passed"])


if __name__ == "__main__":
    unittest.main()
