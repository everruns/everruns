# Individual test review ledger

Run from the repository root after installing the UI dependencies:

```sh
python3 scripts/test-quality/selftest.py
python3 scripts/test-quality/ledger.py sync
python3 scripts/test-quality/ledger.py show --path crates/ard/src/client.rs
python3 scripts/test-quality/ledger.py check
python3 scripts/test-quality/ledger.py summary
```

`sync` reconciles tracked Rust and JavaScript/TypeScript source declarations.
Stage newly added test files before syncing. It never creates review decisions.
`check` detects inventory drift; `summary` reports pending, stale, reviewed and
retired declarations separately. `show` prints every declaration in a source file
in line order for individual review. Read the implementation and shared fixtures
as well as the displayed body.

After reviewing an entry, explicitly add a `review` object to its JSONL row with
`decision` (`keep`, `improved`, or `finding`), a concrete `rationale`, and the
reviewed `body_hash` and `file_hash`. Copy hashes only after actually reviewing
that version. `finding` is unresolved work, not completion. A surrounding-file
change conservatively makes the decision stale, even if the body is unchanged.
Cross-file implementation and fixture changes still require reviewer judgment;
these hashes are not a proof that external dependencies stayed unchanged.

Removed declarations remain as `retired` entries. Give each a `resolution` with
an explicit decision, rationale and replacement ID when applicable. Check that
the retained test actually protects the removed behavior. Historical baseline
notes stay under `prior_review`; they are not evidence of a current review.

The baseline JSONL preserves the original audit at the revision documented in
[the investigation](../../knowledge/evaluation/unit-test-quality.md). Source
candidate counts differ from compiled cases. Macro expansion, custom test
harnesses and runtime-generated cases require separate inspection. This Python
tool's own regression tests are run by the explicit `selftest.py` command.
