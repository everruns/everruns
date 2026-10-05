#!/usr/bin/env python3
"""Regression tests for literal and shared-README crate documentation."""

import unittest

from published_crate_docs import crate_rustdoc


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


if __name__ == "__main__":
    unittest.main()
