# TC001: OKF-Grounded Data Analyst Answer

## Description

Showcase the end-to-end OKF use case: seed a Knowledge Base by importing an OKF
bundle of curated data context (a table doc and a business-metric definition),
then have a Data Analyst agent answer a question that depends on that curated
knowledge — grounding its SQL and interpretation in the imported entries.

This is the headline demo for OKF support and the basis for the announcement
walkthrough/recording. See specs/okf-adoption.md and the
[Share knowledge with OKF](../../../docs/how-to/share-knowledge-with-okf.md) guide.

## Preconditions

- Control-plane running (`just start-dev` or `just start-all`)
- LLM API key configured
- Built-in `data-analyst` harness provisioned (automatic on org init)
- **Agent-side retrieval:** the agent consumes the Knowledge Base via the
  `knowledge_base` capability's `search_knowledge` tool, bound to the KB created
  in step 1.

## Test Data

### OKF bundle (imported into the KB)

`business/active_user.md`:
```markdown
---
type: Metric Definition
title: Active User
tags: [metrics, engagement]
---
An **active user** is one who logged in within the last 30 days. Use
`MAX(last_login_at) >= NOW() - INTERVAL '30 days'` per user. Do not count
soft-deleted accounts (`deleted_at IS NULL`).
```

`tables/users.md`:
```markdown
---
type: BigQuery Table
title: users
resource: https://example.com/users
tags: [users]
---
# Schema
| Column | Type | Notes |
|--------|------|-------|
| `user_id` | STRING | PK |
| `last_login_at` | TIMESTAMP | last successful login |
| `deleted_at` | TIMESTAMP | null unless soft-deleted |
```

### Sample `users` rows (load into SQL)

```
user_id,last_login_at,deleted_at
u1,2026-06-10T00:00:00Z,
u2,2026-06-15T00:00:00Z,
u3,2026-01-01T00:00:00Z,
u4,2026-06-16T00:00:00Z,2026-06-01T00:00:00Z
```

## Steps

1. **Create a Knowledge Base** named "Analytics Knowledge" and **import** the
   bundle above (`POST /v1/knowledge-bases/{kb_id}/okf_import` with the two
   files). Confirm `created: 2`.

2. **Create a Data Analyst agent** ("Analytics Analyst") on the `data-analyst`
   harness, bound to the `knowledge_base` capability with `bases: [<kb_id>]`.

3. **Start a session.** Load the sample `users` rows into a SQL table.

4. **Send message:** "How many active users do we have? Use our definition."

5. **Wait for the agent to finish.**

## Expected Result

- During step 1, the KB has two entries: `Active User` (`kind: business`) and
  `users` (`kind: table`, `resource` set).
- In step 4–5 the agent consults the curated knowledge (via `search_knowledge`)
  **before** writing SQL, applies the imported definition — login within 30 days
  **and** `deleted_at IS NULL` — and answers **2 active users** (`u1`, `u2`;
  `u3` is stale, `u4` is soft-deleted).
- The answer references the "active user" definition it retrieved, not an
  assumed one.

### Failure Modes

| Failure | What to look for |
|---------|-----------------|
| Definition ignored | Agent counts `u4` (soft-deleted) or `u3` (stale) → answers 3 or 4 |
| Knowledge not consulted | No `search_knowledge` call before SQL |
| Import failed | KB has fewer than 2 entries, or wrong kinds |

## Notes

- Deterministic model output is not guaranteed; assert on the retrieval call,
  the SQL predicate, and the final count rather than exact wording.
- The same bundle can be exported (`okf_export`) and committed to git, then
  re-imported to update the KB — "knowledge as code".

## Verified run (dev mode, real Anthropic model)

Captured against `just start-dev` (in-memory) with `claude-sonnet-4-6`,
proving the OKF→agent loop end to end:

1. **Import** — `POST /v1/knowledge-bases/{kb}/okf_import` →
   `{"created":2,"updated":0,"skipped":0,"pruned":0,"warnings":[]}`. Entries:
   `Active User` (`kind: business`, from `type: Metric Definition`) and `users`
   (`kind: table`, `resource` set, raw `type: BigQuery Table` preserved).
2. **Agent** bound to `knowledge_base` (`bases: [<kb>]`) calls `search_knowledge`
   directly (no tool-search detour, thanks to `DeferrablePolicy::Never`). With
   query `"active user"` it returns:
   ```json
   {"count":1,"results":[{"id":"kbe_…","kind":"business","title":"Active User",
     "tags":["metrics","engagement"],
     "snippet":"An **active user** is one who logged in within the last 30 days …
       Exclude soft-deleted accounts (deleted_at IS NULL)."}]}
   ```
3. **Answer** quotes the definition and cites the `kbe_` id.

**Recall caveat (follow-up):** keyword search uses `plainto_tsquery`
(AND-semantics), so an over-specific query like `"active user definition"`
returns nothing while `"active user"` matches. Consider
`websearch_to_tsquery` / an OR fallback to improve agent-driven recall.

