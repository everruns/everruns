#!/usr/bin/env python3
"""Regression tests for crate documentation and public file references."""

import pathlib
import tempfile
import unittest

from published_crate_docs import crate_rustdoc, file_reference_errors


class CrateRustdocTests(unittest.TestCase):
    def test_literal_docs_after_lint_attribute(self):
        valid, docs = crate_rustdoc(
            '#![allow(dead_code)]\n//! https://everruns.com\n//! ```rust\n//! let x = 1;\n//! ```\npub fn f() {}',
            "",
        )
        self.assertTrue(valid)
        self.assertIn("//! ```rust", docs)

    def test_shared_readme_is_validated_as_crate_documentation(self):
        valid, docs = crate_rustdoc(
            '#![allow(dead_code)]\n#![doc = include_str!("../README.md")]\npub fn f() {}',
            '# example\nhttps://everruns.com\n```no_run\nfn main() {}\n```',
        )
        self.assertTrue(valid)
        self.assertIn("https://everruns.com", docs)
        self.assertIn("//! ```no_run", docs)

    def test_shared_readme_does_not_invent_missing_content(self):
        valid, docs = crate_rustdoc('#![doc = include_str!("../README.md")]', "# empty")
        self.assertTrue(valid)
        self.assertNotIn("https://everruns.com", docs)
        self.assertNotIn("```", docs)

    def test_nonleading_include_does_not_count(self):
        for prefix in ["pub fn f() {}", "// ordinary comment"]:
            with self.subTest(prefix=prefix):
                valid, docs = crate_rustdoc(
                    prefix + '\n#![doc = include_str!("../README.md")]',
                    "https://everruns.com\n```rust\n```",
                )
                self.assertFalse(valid)
                self.assertEqual(docs, "")

    def test_item_documentation_does_not_count(self):
        valid, docs = crate_rustdoc(
            '#[doc = include_str!("../README.md")]\npub fn f() {}',
            "https://everruns.com\n```rust\n```",
        )
        self.assertFalse(valid)
        self.assertEqual(docs, "")


class FileReferenceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = pathlib.Path(self.temp.name)
        source = self.root / "crates/core/src/events/mod.rs"
        source.parent.mkdir(parents=True)
        source.touch()

    def test_event_reference_requires_link(self):
        for text in [
            "The event constants are in `crates/core/src/events/mod.rs`.",
            "Use `events` from `crates/core/src/events/mod.rs`.",
        ]:
            with self.subTest(text=text):
                self.assertEqual(file_reference_errors(self.root, text), [
                    (1, "remove unnecessary source path or link it: crates/core/src/events/mod.rs")
                ])

    def test_internal_knowledge_references_are_not_public_links(self):
        self.assertEqual(file_reference_errors(
            self.root, "See [design](knowledge/framework/execution-backends.md)."
        ), [(1, "remove internal knowledge references from public prose")])

    def test_links_and_commands_are_allowed(self):
        text = """See [schema](crates/core/src/events/mod.rs).
Run `cat crates/core/src/events/mod.rs`.
```sh
cat crates/core/src/events/mod.rs
```
~~~rust
// knowledge/framework/execution-backends.md
~~~
Create `/workspace/report.md`.
"""
        self.assertEqual(file_reference_errors(self.root, text), [])

    def test_diagnostic_lines_follow_fenced_code(self):
        text = "```sh\ncat crates/core/src/events/mod.rs\n```\ncrates/core/src/events/mod.rs"
        self.assertEqual(file_reference_errors(self.root, text)[0][0], 4)


if __name__ == "__main__":
    unittest.main()
