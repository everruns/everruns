# Migrations Specification

## Abstract

PostgreSQL schema migrations live in `crates/server/migrations/`. They use SQLx's built-in migration runner and auto-apply on server startup (except in DEV_MODE).

## Naming Convention

```
NNN_<description>.sql
```

- `NNN` — zero-padded 3-digit sequential number (001, 002, ...)
- `<description>` — feature name for development migrations, version tag for squashed releases

Examples:
- `011_evals_target.sql` — feature migration added during development
- `010_v0.8.9.sql` — squashed release migration

## Development Workflow

During development, add migrations with the next sequential number and a descriptive feature name:

```
012_evals_post.sql
013_session_export.sql    ← next feature migration
```

Multiple branches may target the same number. After every rebase onto `origin/main`, check for duplicate numbers and renumber yours to the next available.

## Release Squashing

Before or during a release, all feature migrations added since the last release are **squashed** into a single migration named after the release version:

```
# Before squash (development)
011_evals_target.sql
012_evals_post.sql
013_session_export.sql

# After squash (release v0.8.10)
011_v0.8.10.sql
```

The squashed file contains the combined DDL in execution order, preserving section headers that reference the original migrations for traceability.

Squashing is a **BREAKING CHANGE** — it requires a fresh database. Existing `_sqlx_migrations` rows won't match. This is acceptable because we don't support in-place upgrades across releases yet.

## Sequential Ordering Validation

Migration numbers must be strictly sequential with no gaps and no duplicates. Both `just pre-push` and `/ship` must validate this before pushing or merging.

**Validation rule:** filenames in `crates/server/migrations/` sorted lexicographically must have numbers 001, 002, ..., N with no gaps or repeats.

## Execution

- **Framework:** SQLx `migrate!()` macro (see `crates/server/src/app_builder.rs`)
- **Auto-apply:** on PostgreSQL backend startup (skipped in DEV_MODE, skippable via `--no-migrations`)
- **Tracking table:** `_sqlx_migrations` (version, description, checksum, installed_on)
- **Build integration:** `crates/server/build.rs` watches the migrations directory so binaries rebuild when migrations change

## Conflict Resolution

Migrations are the most common source of merge conflicts because multiple branches often claim the same next number. The fix is always: renumber your migration to the next available number after rebase.

This is called out in:
- `AGENTS.md` (Branch Base section)
- `specs/shipping.md` (Required Outcomes, item 1)
- `.claude/skills/ship/SKILL.md`
- `crates/server/migrations/AGENTS.md`
