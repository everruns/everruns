#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 - <<'PY'
import copy
import sys
import unittest
sys.path.insert(0, 'scripts/codeql')
from verify_rust import verify

class CoverageTest(unittest.TestCase):
    def setUp(self):
        self.metadata = {'workspace_root': '/src', 'workspace_members': ['core'],
                         'packages': [{'id': 'core', 'targets': [{'src_path': '/src/core/lib.rs'}]}]}
        self.rows = [{'path': 'core/lib.rs', 'semantics': 'true', 'nodes': '10'}]
        self.sarif = {'runs': [{'tool': {'driver': {'name': 'CodeQL', 'rules': [{'id': 'rust/sql-injection'}]}},
                                'invocations': [{'executionSuccessful': True}], 'results': []}]}
    def errors(self):
        return verify(self.metadata, self.rows, self.sarif)[1]
    def test_valid_empty_findings(self):
        self.assertEqual([], self.errors())
    def test_missing_crate(self):
        self.metadata['packages'][0]['targets'].append({'src_path': '/src/core/main.rs'})
        self.assertTrue(self.errors())
    def test_indexed_filename_is_not_semantic_coverage(self):
        self.rows[0]['semantics'] = 'false'
        self.assertTrue(self.errors())
    def test_empty_ast(self):
        self.rows[0]['nodes'] = '0'
        self.assertTrue(self.errors())
    def test_failed_execution(self):
        self.sarif['runs'][0]['invocations'][0]['executionSuccessful'] = False
        self.assertTrue(self.errors())
    def test_missing_execution(self):
        self.sarif['runs'][0]['invocations'] = []
        self.assertTrue(self.errors())
    def test_empty_sarif(self):
        self.sarif['runs'] = []
        self.assertTrue(self.errors())
    def test_wrong_language(self):
        self.sarif['runs'][0]['tool']['driver']['rules'] = [{'id': 'py/sql-injection'}]
        self.assertTrue(self.errors())
    def test_extraction_error_even_with_successful_execution(self):
        self.sarif['runs'][0]['invocations'][0]['toolExecutionNotifications'] = [
            {'level': 'error', 'message': {'text': 'workspace loading failed'}}]
        self.assertIn('workspace loading failed', self.errors())
    def test_warnings_retained(self):
        self.sarif['runs'][0]['invocations'][0]['toolExecutionNotifications'] = [
            {'level': 'warning', 'message': {'text': 'macro expansion incomplete'}}]
        self.assertEqual(['macro expansion incomplete'], verify(self.metadata, self.rows, self.sarif)[2])
    def test_findings_unchanged(self):
        self.sarif['runs'][0]['results'] = [{'ruleId': 'rust/sql-injection'}]
        original = copy.deepcopy(self.sarif)
        self.assertEqual([], self.errors())
        self.assertEqual(original, self.sarif)

unittest.main()
PY
