---
type: Investigation
title: Unit-test quality review
description: Evidence and completion criteria for the individual review of the unit-test suite.
tags:
  - testing
  - maintenance
---

# Unit-test quality review

The objective is a useful, efficient regression suite. Each source test declaration
must receive an individual review of its assertions, production path, fixtures,
and overlap with other tests. A heuristic match, passing execution, or coverage
percentage does not establish test quality.

The [ledger](test-quality-ledger.jsonl) is the completion record. Review decisions
require an explicit rationale and identify the test body and surrounding source
that were inspected. Source changes invalidate those decisions. Pending or stale
entries must not be counted as reviewed. Removed declarations remain in the ledger
so deletion cannot silently satisfy the review requirement.

The initial audit screened 8,453 Rust and JavaScript unit-test candidates at
`eb38532714522ae8292e2bd2eb2f5e709ef072cc`. Only 161 received individual review;
8,292 were pending. [PR #3369](https://github.com/everruns/everruns/pull/3369)
addressed 21 confirmed findings and exposed a provider URL error-classification
bug. It did not complete the each-test review. Historical classifications are
preserved in the [original baseline](test-quality-baseline.jsonl), separately
from decisions verified against the current source. Historical review notes are
not automatically promoted to current decisions.

## Review standard

Keep tests that protect observable behavior, independent protocol contracts,
security defaults, error redaction, meaningful boundary cases, or type constraints
that are themselves the contract. Similar setup is not sufficient evidence of
duplication. Different implementations may need the same conformance scenarios.

Replace tests that only reproduce production logic, inspect test-local constants,
assert their own mock's behavior, or tolerate empty output and complete no-ops.
Remove redundant checks when the retained test demonstrably protects the same
behavior. Exercise configuration through its consumer when the use of the setting
is the relevant behavior. Keep network-dependent integration tests out of the unit
suite; make failures deterministic and bounded.

Efficiency includes maintenance and diagnosis as well as execution time. Prefer
small fixtures, deterministic clocks, local transports, and a precise failure
signal. Fewer tests or lines alone are not a success measure.

## Inventory boundary

The source inventory includes Rust test-attributed functions and JavaScript /
TypeScript test declarations. Integration-directory tests and browser E2E cases
are inventoried separately. Parameterized declarations count once; review their
case tables and generators together with the body. These are source candidate
counts, not the number of compiled or executed test cases. Feature gates, macro
expansions, doctests, and shell test harnesses require separate reachability or
execution evidence and must not be represented as covered by declaration counts.

The executable inventory and reconciliation commands live in
[`scripts/test-quality/`](../../scripts/test-quality/). The TypeScript parser
uses the UI's installed dependency. Inventory refresh never creates a review
decision. The review is complete only when all current unit candidates have a
current decision, removals have an explicit disposition, in-scope fixes are
validated and shipped, and no unresolved finding is disguised as completion.
