# Pre-Release Maintenance Specification

## Abstract

This specification defines the pre-release maintenance checklist for Everruns. Run before each release to ensure dependencies, specs, security posture, test coverage, documentation, and agent guidance are current.

## Checklist

### 1. Dependencies

Update all dependencies to latest versions (including major).

**Backend (Rust):**
1. Run `cargo update` to update `Cargo.lock`
2. Review `[workspace.dependencies]` in root `Cargo.toml` for outdated pinned versions
3. Run `cargo outdated` (or manually check crates.io) for major version bumps
4. Apply major version updates; fix breaking changes
5. Run `cargo clippy --all-targets --all-features -- -D warnings` — must pass
6. Run `cargo test --all-features` — must pass

**UI (apps/ui):**
1. Run `npm outdated` in `apps/ui/`
2. Update `package.json` dependencies (including major)
3. Run `npm install` to regenerate `package-lock.json`
4. Run `npm run lint` and `npm run build` — must pass
5. Run `npm test` — must pass
6. Treat smoke-test screenshots as review artifacts only; do not commit generated screenshot files to the repo

**Docs (apps/docs):**
1. Run `npm outdated` in `apps/docs/`
2. Update `package.json` dependencies
3. Run `npm install` to regenerate lock file
4. Run `npm run build` — must pass

### 2. Specs Accuracy

Verify all specs in `specs/` reflect current code.

1. List all spec files and compare against implemented features
2. Check each spec's data models against actual Rust structs and DB schema
3. Check API spec (`specs/apis.md`) against OpenAPI output (`./scripts/export-openapi.sh`)
4. Verify event types in `specs/events.md` match emitted events in code
5. Verify capability list in `specs/capabilities.md` matches registered capabilities
6. Verify MCP server spec matches implementation
7. Remove specs for features that no longer exist
8. Add specs for features not yet documented
9. Ensure `AGENTS.md` specs list is complete (all files in `specs/` are listed)

### 3. Threat Model

Update `specs/threat-model.md` for current state.

1. Review all OPEN threats — check if any have been mitigated since last review
2. Review ACCEPTED risks — verify rationale still holds
3. Check for new attack surface from recent features
4. Verify `THREAT[TM-XXX-NNN]` code comments exist at mitigation points
5. Update vulnerability summary tables
6. Review caller responsibilities — still accurate?

### 4. Test Coverage

Identify and fill gaps.

1. Run `cargo test --all-features` — all must pass
2. Check each crate has tests for public API surface
3. Verify integration tests cover critical paths (auth, agent CRUD, session lifecycle, durable workflows)
4. Check UI tests: `npm test` in `apps/ui/`
5. Verify manual test cases in `test_cases/` are current
6. Check for untested error paths and edge cases
7. New features since last release must have test coverage

### 5. Performance Review

Verify no new performance regressions, especially in database access patterns.

1. Review new/changed SQL queries and ORM calls for full table scans — all list queries must use indexes
2. Check for N+1 query patterns (loading related entities in a loop instead of a single join/batch)
3. Verify all list endpoints have pagination with bounded `LIMIT`; no unbounded `SELECT *` without `WHERE`/`LIMIT`
4. Check new indexes have corresponding migrations; verify with `EXPLAIN ANALYZE` on representative data
5. Review any new background jobs or scheduled tasks for query cost at scale
6. Ensure bulk operations (batch inserts, deletes, updates) are bounded and chunked where appropriate
7. Check for missing database indexes on columns used in `WHERE`, `JOIN`, or `ORDER BY` clauses
8. Verify no hot-path code holds long-lived DB connections or transactions

### 6. API Documentation

1. Run `./scripts/export-openapi.sh` — must succeed
2. Verify OpenAPI spec at `docs/api/openapi.json` is up to date
3. Check all endpoints have descriptions and response types in utoipa annotations
4. Cross-check `specs/apis.md` with actual routes

### 7. Examples

1. Verify all example markdown files in `examples/` have valid agent definitions
2. Check `examples/hackernews-reader/` README is accurate
3. Verify `examples/docker-compose-full.yaml` works with current Docker image
4. Run smoke test against example agents if possible

### 8. README

1. Verify Quick Start instructions work
2. Check all links (docs, badges, API reference) resolve
3. Ensure feature list is current
4. Verify API example matches current API shape
5. Check Docker Compose instructions match `examples/docker-compose-full.yaml`

### 9. SDK Documentation and Feature Parity

Verify [everruns/sdk](https://github.com/everruns/sdk) public documentation (`docs/features/sdk.mdx`) is current and the SDK supports latest main API features.

**Documentation accuracy:**
1. Compare SDK docs API coverage table against actual endpoints in `specs/apis.md`
2. Verify documented sub-clients (agents, sessions, messages, events) match server routes
3. Check documented event types match `specs/events.md`
4. Verify code examples use current request/response shapes (compare against OpenAPI spec)
5. Confirm installation instructions and version references are current

**Feature parity:**
6. Compare SDK version in `Cargo.toml` (`everruns-sdk`) against latest published release
7. Check if new API resources added since last SDK release are missing from SDK (e.g., MCP servers, LLM providers, scheduled tasks, session databases, organizations)
8. Verify CLI crate (`crates/cli/`) can exercise all SDK sub-clients without errors
9. File issues or PRs on [everruns/sdk](https://github.com/everruns/sdk) for any gaps found

**SDK publishing:**
10. Verify all SDK languages are published to their registries for the latest tag:
    - **npm**: `npm view @everruns/sdk versions` — TODO: versions beyond 0.1.0 are not yet published
    - **PyPI**: `pip index versions everruns-sdk` — all versions published
    - **crates.io**: `cargo search everruns-sdk` — all versions published
11. The CI SDK compatibility matrix resolves versions per registry. Unpublished versions are automatically excluded but reduce test coverage for that language.

### 10. Rust Documentation

1. Run `cargo doc --no-deps --all-features` — must compile without warnings
2. Verify public types have doc comments
3. Check crate-level documentation (`//!` comments in `lib.rs`)
4. Fix any broken intra-doc links

### 11. AGENTS.md (CLAUDE.md)

`AGENTS.md` is the primary agent instruction file (`CLAUDE.md` references it).

1. Verify specs list matches files in `specs/`
2. Verify skills list matches directories in `.claude/skills/`
3. Check pre-PR checklist is current
4. Verify local dev commands work
5. Verify cloud agent start commands work
6. Check commit convention matches `commitlint.config.js`
7. Ensure tone/style guidance is clear and consistent

### 12. Additional Checks

1. `cargo deny check` — license compliance passes
2. `cargo fmt --check` — formatting passes
3. CI workflows (`.github/workflows/`) are current
4. Docker build succeeds: `docker build .`
5. No TODO/FIXME items that should be resolved before release
6. CHANGELOG.md has entries for all changes since last release

### 13. Spec Hygiene — Avoid Code Duplication

Specs should capture design intent, rationale, and constraints — not duplicate what's readable from code. Code-derivable details drift out of sync and create maintenance burden.

**What to remove or replace with links:**
- Exhaustive struct field listings → link to the Rust source file
- Exact enum variant lists → link to the enum definition
- Full API request/response JSON shapes → reference OpenAPI spec or handler code
- SQL DDL / migration contents → link to migration files
- Config file formats reproduced verbatim → link to the config file or example
- Function signatures or trait method listings → link to the trait definition

**What to keep in specs:**
- Design rationale ("why this approach over alternatives")
- Architectural constraints and invariants
- Relationships between concepts (how pieces fit together)
- Security considerations and threat boundaries
- Behavioral contracts not obvious from types alone
- Diagrams showing data/control flow

**Link format:** Use relative paths from repo root, e.g., "See `crates/core/src/models/agent.rs` for full field list."

**Review process:**
1. For each spec, check whether sections could be replaced by "see `path/to/file.rs`"
2. Keep the *purpose* and *constraints* of each field/endpoint; remove the exhaustive listing
3. Ensure any remaining examples are illustrative (showing pattern), not exhaustive (listing all cases)

### 14. Code Simplification

Run the [code-simplifier](https://github.com/anthropics/claude-plugins-official/blob/main/plugins/code-simplifier/agents/code-simplifier.md) agent against recently modified code to clean up clarity, consistency, and maintainability issues while preserving exact functionality.

1. Review code changed since last release (`git diff <last-release-tag>..HEAD --stat`)
2. Run code-simplifier agent on changed files — focuses on:
   - Deduplicating repeated patterns into shared helpers
   - Consolidating split/scattered imports
   - Removing redundant code (unused imports, passthrough wrappers, duplicate logic)
   - Applying project conventions consistently (naming, error handling, type annotations)
   - Simplifying overly nested or complex expressions
3. Verify all tests still pass after simplification
4. Ensure no behavioral changes — only structural improvements



### 15. Docs Site SEO & Crawlability

Verify the documentation site at `docs.everruns.com` is healthy for search engines and users.

1. Build docs: `npm run build` in `apps/docs/` — must succeed without warnings
2. Verify sitemap: `sitemap.xml` includes all published pages (currently 128+); no missing entries
3. Check for orphaned pages: all built HTML pages must be reachable via sidebar navigation or cross-links — pages hidden from sidebar (e.g., commented-out SRE section) should either be restored to navigation or excluded from build
4. Verify `robots.txt` allows all crawlers and points to correct sitemap URL
5. Meta descriptions: spot-check 5+ pages (homepage, introduction, API operation, feature page) for `<meta name="description">` presence
6. Page titles: verify titles are ≤70 characters and properly formatted (no raw `METHOD /path` prefixes leaking)
7. Trailing slash consistency: confirm Astro `trailingSlash` setting aligns with hosting platform (Cloudflare Pages enforces trailing slashes via 308 redirects for directory-style URLs — this is expected behavior, not a bug)
8. Internal links: spot-check navigation links resolve without unexpected redirects or 404s
9. Search index: verify Pagefind search works on the built site

Run `just pre-pr` to automate checks 1-10 where possible. Manual review needed for spec accuracy, threat model, performance review, SDK parity, AGENTS.md content, and docs site SEO.

## Frequency

Run full checklist before each release. Individual sections can be run independently during development.
