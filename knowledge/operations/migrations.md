---
type: Specification
title: "Migrations Specification"
description: "Database migration naming, squashing, ordering, conflict resolution."
tags:
  - everruns
  - operations
---
# Migrations Specification

## Abstract

PostgreSQL schema migrations live in `crates/server/migrations/`. They use SQLx's built-in migration runner and auto-apply on server startup (except in DEV_MODE).

## Naming Convention

```
NNN_<description>.sql
```

- `NNN`, zero-padded 3-digit sequential number (001, 002, ...)
- `<description>`, descriptive migration name; historical version tags remain valid in older files, but new migrations should use feature names

Examples:
- `011_evals_target.sql`, feature migration added during development
- `010_v0.8.9.sql`, historical release-named migration kept for compatibility

## Development Workflow

During development, add migrations with the next sequential number and a descriptive feature name:

```
012_evals_post.sql
013_session_export.sql    ← next feature migration
```

Multiple branches may target the same number. After every rebase onto `origin/main`, check for duplicate numbers and renumber yours to the next available.

## Release Handling

Release preparation does not squash feature migrations into a version-named file. Ship migrations as authored.

Do not rename, rewrite, or delete an existing migration just to match a release version. SQLx records each migration's version, description, and checksum in `_sqlx_migrations`; changing a migration that may already have been applied breaks startup against databases that recorded the original file.

Historical `NNN_vX.Y.Z.sql` files remain valid and must stay unchanged, but new releases do not require creating one. If a release needs new schema work, land it as a normal sequential feature migration before cutting the release.

`crates/server/tests/migration_history_test.rs` locks specific historical filenames and SQL bodies that must not change.

## Sequential Ordering Validation

Migration numbers must be strictly sequential with no gaps and no duplicates. `just pre-push`, `just pre-pr`, and `/ship` (after rebase and again immediately before merge) all validate this.

**Validation rule:** filenames in `crates/server/migrations/` sorted lexicographically must have numbers 001, 002, ..., N with no gaps or repeats.

**Validator:** `scripts/lib/check-migration-ordering.sh` is the single source of truth. It is invoked by `pre-push.sh`, `pre-pr.sh`, and the `/ship` skill.

## Execution

- **Framework:** SQLx `migrate!()` macro (see `crates/server/src/app_builder.rs`)
- **Auto-apply:** on PostgreSQL backend startup (skipped in DEV_MODE, skippable via `--no-migrations`)
- **Tracking table:** `_sqlx_migrations` (version, description, checksum, installed_on)
- **Build integration:** `crates/server/build.rs` watches the migrations directory so binaries rebuild when migrations change

## Conflict Resolution

Migrations are the most common source of merge conflicts because multiple branches often claim the same next number. The fix is always: renumber your migration to the next available number after rebase.

`scripts/lib/check-migration-ordering.sh` is the enforcement point, run after every rebase and
again immediately before merge. It is wired into `just pre-push`, `just pre-pr`, and `/ship`.
