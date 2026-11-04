#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
python3 - <<'PY'
import copy
import os
from pathlib import Path
import subprocess
import tempfile
import yaml
import sys
import unittest
sys.path.insert(0, 'scripts/codeql')
from verify_rust import verify

class CoverageTest(unittest.TestCase):
    def setUp(self):
        self.metadata = {'workspace_root': '/src', 'workspace_members': ['core'],
                         'packages': [{'id': 'core', 'targets': [{'src_path': '/src/core/lib.rs'}]}]}
        self.rows = [{'path': 'core/lib.rs', 'semantics': 'true', 'nodes': '10', 'crates': '1'}]
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
    def test_ast_without_crate_graph_is_not_target_coverage(self):
        self.rows[0]['crates'] = '0'
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
    def test_action_rules_grouped_by_pack(self):
        tool = self.sarif['runs'][0]['tool']
        tool['extensions'] = [{'name': 'codeql/rust-queries', 'rules': tool['driver'].pop('rules')}]
        self.assertEqual([], self.errors())
    def test_grouped_wrong_language(self):
        tool = self.sarif['runs'][0]['tool']
        tool['driver'].pop('rules')
        tool['extensions'] = [{'rules': [{'id': 'py/sql-injection'}]}]
        self.assertTrue(self.errors())
    def test_extraction_error_even_with_successful_execution(self):
        self.sarif['runs'][0]['invocations'][0]['toolExecutionNotifications'] = [
            {'level': 'error', 'message': {'text': 'workspace loading failed'}}]
        self.assertIn('workspace loading failed', self.errors())
    def test_warnings_retained(self):
        self.sarif['runs'][0]['invocations'][0]['toolExecutionNotifications'] = [
            {'level': 'warning', 'message': {'text': 'macro expansion incomplete'}}]
        self.assertEqual(['macro expansion incomplete'], verify(self.metadata, self.rows, self.sarif)[2])
    def test_sarif_default_warning_level_retained(self):
        self.sarif['runs'][0]['invocations'][0]['toolExecutionNotifications'] = [
            {'message': {'text': 'implicit warning'}}]
        self.assertEqual(['implicit warning'], verify(self.metadata, self.rows, self.sarif)[2])
    def test_findings_unchanged(self):
        self.sarif['runs'][0]['results'] = [{'ruleId': 'rust/sql-injection'}]
        original = copy.deepcopy(self.sarif)
        self.assertEqual([], self.errors())
        self.assertEqual(original, self.sarif)

class StorageTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.steps = yaml.safe_load(Path('.github/codeql/advanced.yml').read_text())['jobs']['rust']['steps']
        cls.script = next(step['run'] for step in cls.steps if step.get('id') == 'storage')

    def prepare(self, filesystem, status=0):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fake_stat = root / 'stat'
            fake_stat.write_text('#!/bin/sh\nprintf "%s\\n" "$TEST_FILESYSTEM"\nexit "$TEST_STAT_STATUS"\n')
            fake_stat.chmod(0o755)
            env = dict(os.environ, PATH=str(root)+':'+os.environ['PATH'],
                       TEST_FILESYSTEM=filesystem, TEST_STAT_STATUS=str(status),
                       RUNNER_TEMP=directory, GITHUB_RUN_ID='123', GITHUB_RUN_ATTEMPT='2',
                       GITHUB_ENV=str(root/'env'), GITHUB_OUTPUT=str(root/'output'))
            result = subprocess.run(['bash', '-e', '-o', 'pipefail', '-c', self.script],
                                    env=env, text=True, capture_output=True)
            return result.returncode, (root/'env').read_text() if (root/'env').exists() else ''

    def test_disk_storage_exports_build_and_temporary_paths(self):
        status, env = self.prepare('ext2/ext3')
        self.assertEqual(status, 0)
        self.assertIn('CODEQL_EXTRACTOR_RUST_OPTION_CARGO_TARGET_DIR=', env)
        self.assertIn('TMPDIR=', env)

    def test_tmpfs_does_not_consume_analysis_ram(self):
        status, env = self.prepare('tmpfs')
        self.assertNotEqual(status, 0)
        self.assertEqual(env, '')

    def test_ramfs_also_rejected(self):
        self.assertNotEqual(self.prepare('ramfs')[0], 0)

    def test_unknown_storage_fails_closed(self):
        self.assertNotEqual(self.prepare('', status=1)[0], 0)

    def test_findings_are_not_published_in_public_artifacts(self):
        upload = next(step for step in self.steps if step.get('uses', '').startswith('actions/upload-artifact@'))
        self.assertEqual(set(upload['with']['path'].splitlines()),
                         {'evidence/workspace.json', 'evidence/files.csv', 'evidence/resources.txt'})

    def extract(self, fail_stage=''):
        script = next(step['run'] for step in self.steps if step.get('id') == 'extract')
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cache = root / 'codeql-rust-123-2/cargo-target'
            cache.mkdir(parents=True)
            (cache / 'compiled').write_text('build cache')
            other = root / 'another-run/cargo-target'
            other.mkdir(parents=True)
            codeql = root / 'codeql'
            codeql.write_text('#!/bin/sh\n[ "$2" != "$FAIL_STAGE" ]\n')
            codeql.chmod(0o755)
            env = dict(os.environ, CODEQL=str(codeql), DATABASE=str(root/'databases/rust'),
                       RUNNER_TEMP=directory, GITHUB_RUN_ID='123', GITHUB_RUN_ATTEMPT='2',
                       FAIL_STAGE=fail_stage)
            result = subprocess.run(['bash', '-e', '-o', 'pipefail', '-c', script],
                                    env=env, capture_output=True)
            return result.returncode, cache.exists(), other.exists()

    def test_success_releases_only_this_runs_build_cache(self):
        self.assertEqual(self.extract(), (0, False, True))

    def test_extraction_failure_keeps_cache_and_fails_step(self):
        status, cache, other = self.extract('trace-command')
        self.assertNotEqual(status, 0)
        self.assertTrue(cache and other)

    def test_finalization_failure_keeps_cache_and_fails_step(self):
        status, cache, other = self.extract('finalize')
        self.assertNotEqual(status, 0)
        self.assertTrue(cache and other)

unittest.main()
PY
