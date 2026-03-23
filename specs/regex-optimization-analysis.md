# Regex Search Optimization Analysis

Analysis of [Cursor's Fast Regex Search](https://cursor.com/blog/fast-regex-search) ideas
and applicability to everruns.

## Cursor's Key Techniques

1. **Trigram inverted index** — decompose file content into 3-char sequences, build posting
   lists mapping trigram → files. At query time, extract trigrams from the regex, intersect
   posting lists to narrow candidates, then run full regex only on candidates.

2. **Probabilistic masking** (GitHub Blackbird) — augment trigrams with 8-bit bloom filters
   (`locMask` for position, `nextMask` for following char) to get near-quadgram specificity
   at trigram storage cost.

3. **Sparse n-grams** (ClickHouse/GitHub) — deterministic weight function on character pairs
   (CRC32 or frequency-based), extract variable-length n-grams at weight maxima. Fewer
   posting-list lookups than pure trigrams.

4. **Client-side mmap index** — two-file structure (lookup table + postings), only lookup
   table is mmap'd. Index state keyed to git commits with runtime change layers.

## Everruns Search Architecture

| Component | Mechanism | Hot Path |
|-----------|-----------|----------|
| `grep_session_files` (DB) | `convert_from(content, 'UTF8') ~ $pattern` — PostgreSQL regex on every row | YES |
| `grep_session_files` (service) | Two-phase: DB filters files, Rust `regex` does line-level matching | YES |
| Virtual bash `SearchProvider` | Bridges to `grep_session_files` via sync→async shim | YES |
| Full-text message search | `tsvector` + GIN index via `plainto_tsquery` | YES (already optimized) |
| Skill argument substitution | Regexes cached with `LazyLock` and reused | Low frequency |
| Directory listing | `path ~ '^/prefix/[^/]+$'` in PostgreSQL | Low frequency |

## Applicable Ideas

### 1. PostgreSQL `pg_trgm` GIN Index (HIGH VALUE)

The single highest-impact optimization. PostgreSQL already ships `pg_trgm` — the database
equivalent of what Cursor built from scratch.

**Current problem:** `convert_from(content, 'UTF8') ~ $pattern` does a sequential scan on
every file's content per session. No index is used for content matching.

**Proposed fix:**

```sql
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- GIN trigram index on file content for regex acceleration
CREATE INDEX idx_session_files_content_trgm
    ON session_files
    USING GIN ((convert_from(content, 'UTF8')) gin_trgm_ops)
    WHERE is_directory = FALSE;
```

PostgreSQL's `pg_trgm` GIN index automatically decomposes content into trigrams and
accelerates `~` (regex), `~*` (case-insensitive regex), `LIKE`, and `ILIKE` operators.
The query planner uses the index to narrow candidate rows before running the full regex —
exactly the two-phase approach Cursor implemented.

**Expected impact:** For sessions with many files, grep goes from O(n) sequential scan to
O(k) where k is candidate files matching the trigram filter. Biggest win for selective
patterns (e.g., `functionName` vs `.`).

**Trade-off:** Index storage overhead (~1-3x content size). Acceptable for session files
which are bounded in practice.

**Action:** Add migration, benchmark with representative session sizes. Verify the query
planner actually uses the index with `EXPLAIN ANALYZE`.

### 2. Cache Compiled Regexes with `LazyLock` (LOW-HANGING FRUIT)

`expand_skill_arguments()` in `crates/core/src/skill.rs` compiles
`\$ARGUMENTS\[([0-9]+)\]` on every invocation. Use `std::sync::LazyLock`:

```rust
use std::sync::LazyLock;

static INDEXED_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"\$ARGUMENTS\[([0-9]+)\]").unwrap()
});
```

Same for `preprocess_command_injections()` (``!`command` `` syntax).

**Impact:** Marginal (microseconds per call), but it's a clean improvement and consistent
with patterns already used elsewhere in the codebase (`LazyLock` in `virtual_bash.rs`).

### 3. Path-Level Trigram Index (MODERATE VALUE)

Directory listing uses `path ~ '^prefix/[^/]+$'` which also benefits from `pg_trgm`:

```sql
CREATE INDEX idx_session_files_path_trgm
    ON session_files
    USING GIN (path gin_trgm_ops);
```

Less impactful than content indexing since path strings are short and the existing
`idx_session_files_parent` substring index already helps. Worth benchmarking.

### 4. Expression Index on `convert_from` (MODERATE VALUE)

Even without `pg_trgm`, a B-tree expression index could help if content is frequently
compared as text:

```sql
CREATE INDEX idx_session_files_content_text
    ON session_files ((convert_from(content, 'UTF8')))
    WHERE is_directory = FALSE;
```

But this only helps equality/range checks, not regex. `pg_trgm` is strictly better for
the grep use case.

## Not Applicable

| Cursor Technique | Why Not Applicable |
|------------------|--------------------|
| Client-side mmap index | Files live in PostgreSQL, not on disk. DB-native indexing is the right approach. |
| Sparse n-grams | Overkill for session-scoped file sets (hundreds, not millions of files). `pg_trgm` trigrams suffice. |
| Probabilistic masking | Same — useful at GitHub/Chromium scale, not session-scoped virtual filesystems. |
| Git-commit-based freshness | No git in virtual filesystem. PostgreSQL transactions handle consistency. |
| Suffix arrays | Same scale mismatch. |

## Recommendations (Priority Order)

1. **Add `pg_trgm` GIN index on `session_files.content`** — biggest bang for buck, direct
   analog of Cursor's core idea, zero application code changes needed.
2. **Cache compiled regexes** in `skill.rs` with `LazyLock` — trivial, clean.
3. **Benchmark** the grep hot path before and after to quantify improvement.
4. **Monitor** `pg_trgm` index size in production to ensure storage overhead is acceptable.

## Conclusion

The core insight from Cursor's blog — "use trigram indexes to filter candidates before
running expensive regex" — maps directly to PostgreSQL's `pg_trgm` extension. This is the
one optimization worth implementing. The more exotic techniques (sparse n-grams,
probabilistic masking, mmap'd posting files) solve scale problems we don't have since
our search scope is per-session virtual filesystems, not entire codebases.
