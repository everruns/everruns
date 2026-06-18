-- OKF adoption: add `resource` URI to knowledge entries.
-- See specs/okf-adoption.md (Gap Closure).
--
-- OKF concept documents carry an optional `resource` frontmatter field: a URI
-- uniquely identifying the underlying asset (e.g. a BigQuery table console URL).
-- We surface it on entries and use it as the primary idempotency key for OKF
-- imports, falling back to the bundle-relative source path when absent.
--
-- Nullable and non-unique: OKF does not require uniqueness, and two concepts may
-- legitimately reference the same asset.

ALTER TABLE knowledge_entries ADD COLUMN resource TEXT;

COMMENT ON COLUMN knowledge_entries.resource IS 'Optional OKF resource URI identifying the underlying asset. See specs/okf-adoption.md.';
