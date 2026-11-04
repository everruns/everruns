"""Unit checks for offline MCP tool retrieval evaluation."""
import unittest

from evaluate import all_required_recall_at_8, load_cases, ranking_metrics
from retrieval import rank_bm25, rank_everruns


class RetrievalTests(unittest.TestCase):
    def setUp(self):
        self.tools = [
            {"name": "get_invoice", "description": "Read one customer invoice", "inputSchema": {"type": "object"}, "tool_id": "tool-1"},
            {"name": "search_logs", "description": "Search application log records", "inputSchema": {"type": "object"}, "tool_id": "tool-2"},
            {"name": "send_email", "description": "Send a message to a recipient", "inputSchema": {"type": "object"}, "tool_id": "tool-3"},
        ]

    def test_everruns_baseline_matches_name_weight_and_exact_bonus(self):
        results = rank_everruns(self.tools, "get invoice")
        self.assertEqual(results[0]["name"], "get_invoice")
        self.assertEqual(rank_everruns(self.tools, "zzzz"), [])
        self.assertLessEqual(len(rank_everruns(self.tools, "invoice email")), 8)
        self.assertEqual(rank_everruns(self.tools, "!!!"), self.tools)
        self.assertEqual(rank_everruns(self.tools, "“get_invoice”!" )[0]["name"], "get_invoice")

    def test_bm25_returns_original_tool_with_metadata(self):
        results = rank_bm25(self.tools, "application logs")
        self.assertEqual(results[0]["tool_id"], "tool-2")
        self.assertEqual(results[0]["inputSchema"], {"type": "object"})

    def test_optional_aliases_change_retrieval_vocabulary(self):
        tool = {"name": "lookup_customer", "description": "Read a customer profile", "aliases": ["account holder"], "inputSchema": {"type": "object"}}
        tools = [tool, {"name": "other", "description": "unrelated", "inputSchema": {"type": "object"}}]
        self.assertEqual(rank_bm25(tools, "account holder"), [])
        self.assertEqual(rank_bm25(tools, "account holder", include_aliases=True)[0]["name"], "lookup_customer")

    def test_server_description_disambiguates_identical_tools(self):
        tools = [
            {"name": "list_records", "description": "List records", "server": "alpha",
             "server_description": "Payroll compensation and employee benefits"},
            {"name": "list_records", "description": "List records", "server": "beta",
             "server_description": "Geothermal drilling telemetry"},
        ]
        self.assertEqual(rank_bm25(tools, "geothermal")[0]["server"], "beta")
        self.assertEqual(rank_bm25(tools, "payroll")[0]["server"], "alpha")
        self.assertEqual(rank_everruns(tools, "geothermal"), [])

    def test_missing_server_description_preserves_tool_search(self):
        self.assertEqual(rank_bm25(self.tools, "customer invoice")[0]["tool_id"], "tool-1")

    def test_cases_are_varied_and_include_negatives(self):
        cases = load_cases()
        self.assertGreaterEqual(len(cases), 24)
        self.assertGreaterEqual(sum(not case["expected"] for case in cases), 3)
        self.assertTrue(any(case["domain"] == "cross_domain" for case in cases))
        self.assertTrue(any(case["domain"] == "near_duplicate" for case in cases))

    def test_cross_domain_metric_requires_every_tool_in_top_eight(self):
        cases = [{"id": "cross", "query": "invoice logs", "required_tools": ["get_invoice", "search_logs"]}]
        metric = all_required_recall_at_8(cases, self.tools, rank_bm25)
        self.assertEqual(metric, {"cases": 1, "score": 1.0, "all_tools_recalled_cases": 1})

    def test_metrics_keep_negatives_separate_from_recall(self):
        cases = [
            {"id": "hit", "query": "invoice", "expected": "get_invoice"},
            {"id": "miss", "query": "email", "expected": "missing"},
            {"id": "none", "query": "nonsense", "expected": None},
        ]
        metrics = ranking_metrics(cases, self.tools, rank_bm25)
        self.assertEqual(metrics["positive_cases"], 2)
        self.assertEqual(metrics["negative_cases"], 1)
        self.assertEqual(metrics["recall_at_1"], 0.5)
        self.assertEqual(metrics["mrr"], 0.5)
        self.assertEqual(metrics["negative_false_positive_count"], 0)


if __name__ == "__main__":
    unittest.main()
