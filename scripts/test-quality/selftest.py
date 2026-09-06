"""Inventory safety checks. Run with python3 scripts/test-quality/selftest.py."""
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

from ledger import ROOT, digest, read_ledger, reconcile, rust_tests, status


class InventorySafety(unittest.TestCase):
    def test_rust_ignores_fake_declarations_in_comments_and_raw_strings(self):
        source = '''// #[test] fn fake_comment() {}
/* nested /* #[test] fn fake_nested() {} */ comment */
const TEXT: &str = r###" #[test] fn fake_literal() {} "###;
mod tests {
 #[tokio::test(flavor = "multi_thread")]
 async fn real() { assert_eq!("}", r#"{"#); }
}
'''
        rows = list(rust_tests('src/lib.rs', source))
        self.assertEqual([r['name'] for r in rows], ['tests::real'])
        self.assertIn('assert_eq!', rows[0]['body'])
        self.assertTrue(rows[0]['body'].endswith('}'))

    def test_stacked_parameter_attributes_count_one_function(self):
        rows = list(rust_tests('src/lib.rs', '''
#[test_case(Foo { x: 1 })]
#[test_case(Foo { x: 2 })]
fn cases(value: Foo) { assert!(value.x > 0); }
#[rstest::rstest]
#[case(3)]
fn other() {}
'''))
        self.assertEqual([r['name'] for r in rows], ['cases', 'other'])

    def test_attribute_does_not_attach_to_a_later_function(self):
        rows = list(rust_tests('src/lib.rs', '#[test] mod not_a_function {} fn helper() {}'))
        self.assertEqual(rows, [])

    def test_same_name_in_different_modules_is_preserved(self):
        rows = list(rust_tests('src/lib.rs', 'mod a { #[test] fn same() {} } mod b { #[test] fn same() {} }'))
        self.assertEqual([r['name'] for r in rows], ['a::same', 'b::same'])

    def test_reconciliation_never_marks_an_unreviewed_test_as_reviewed(self):
        row = dict(id='a', scope='unit', body_hash=digest('body'), file_hash=digest('file'))
        self.assertEqual(status(reconcile([row], [])[0]), 'pending')

    def test_body_or_context_change_invalidates_review(self):
        row = dict(id='a', scope='unit', body_hash='body', file_hash='file',
                   review=dict(decision='keep', body_hash='body', file_hash='file', rationale='boundary'))
        self.assertEqual(status(row), 'keep')
        self.assertEqual(status(dict(row, body_hash='changed')), 'stale')
        self.assertEqual(status(dict(row, file_hash='changed')), 'stale')

    def test_deletion_is_retained_and_reappearance_is_not_retired(self):
        row = dict(id='a', scope='unit', body_hash='body', file_hash='file')
        retired = reconcile([], [row])
        self.assertEqual(status(retired[0]), 'retired')
        active = reconcile([row], retired)
        self.assertEqual(status(active[0]), 'pending')

    def test_typescript_parser_handles_each_nested_callbacks_and_suites(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'sample.test.ts'
            path.write_text('''
const example = "test('fake', () => {})";
describe('outer', () => {
 test.each([[1, () => ({})], [2, () => ({})]])('case %s', (n, fn) => { expect(fn()).toEqual({}); });
 it('expression', () => expect(1).toBe(1));
 test.todo('unfinished');
});
test.describe('browser', () => { test('actual', async () => {}); });
''')
            rows = json.loads(subprocess.check_output(['node', str(ROOT/'scripts/test-quality/javascript.cjs')], input=json.dumps([str(path)]), text=True))
            self.assertEqual([r['name'] for r in rows], ['outer > case %s', 'outer > expression', 'outer > unfinished', 'browser > actual'])
            self.assertTrue(rows[0]['parameterized'])
            self.assertTrue(rows[2]['disabled'])

    def test_invalid_source_cannot_silently_shrink_inventory(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'broken.test.ts'
            path.write_text("test('broken', () => {")
            result = subprocess.run(['node', str(ROOT/'scripts/test-quality/javascript.cjs')],
                                    input=json.dumps([str(path)]), text=True, capture_output=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('Cannot inventory', result.stderr)

    def test_duplicate_ids_and_unsupported_review_claims_are_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory)/'ledger.jsonl'
            for rows in [
                [dict(id='a'), dict(id='a')],
                [dict(id='a', review=dict(decision='complete'))],
                [dict(id='a', review=dict(decision='keep', body_hash='x', file_hash='y', rationale=' '))],
            ]:
                path.write_text(''.join(json.dumps(row)+'\n' for row in rows))
                with self.assertRaises(ValueError):
                    read_ledger(path)


if __name__ == '__main__':
    unittest.main()
