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

PostgreSQL schema migrations live in `crates/server/migrations/`. They use SQLx's built-in migration runner and auto-apply on server startup, including DEV_MODE, which migrates a fresh embedded database.

## Naming Convention

```
YYYYMMDDHHMMSS_<description>.sql
```

- `YYYYMMDDHHMMSS`, UTC creation time, which becomes the sqlx version. This is sqlx's own default naming.
- `<description>`, lowercase feature name (letters, digits, underscores).
- Create one with `just new-migration <description>`.

Files `001_*.sql` through `220_*.sql` are the frozen sequential history from before the switch
(2026-10-11). They keep their names and versions 1..220, every timestamp sorts after them, and no
new `NNN_` file is accepted.

### Why timestamps

Sequential numbers made every open branch pick "highest on main + 1". Each PR passed CI alone, and
parallel merges landed one number two or three times (215 twice, 218 three times on 2026-10-11).
Two files with one version fail sqlx startup, and the repair (renaming a merged file) breaks any
database that already applied the old name, since sqlx then sees a different checksum for that
version (`VersionMismatch`). Creation timestamps make collisions practically impossible.

sqlx applies every version missing from `_sqlx_migrations`, in version order, without requiring
new versions to be above the highest applied one. A branch that merges after a newer-timestamped
one still applies. The cost is that two environments may apply two parallel migrations in a
different order, which only matters when both change the same objects in dependent ways; review
such pairs as one change.

## Development Workflow

Run `just new-migration <description>` and write the SQL. After a rebase there is nothing to
renumber. If a rebase pulls in a migration your branch depends on that is newer than yours, create
your migration again so it sorts after it.

## Release Handling

Release preparation does not squash feature migrations into a version-named file. Ship migrations as authored.

Do not rename, rewrite, or delete an existing migration just to match a release version. SQLx records each migration's version, description, and checksum in `_sqlx_migrations`; changing a migration that may already have been applied breaks startup against databases that recorded the original file.

Historical `NNN_vX.Y.Z.sql` files remain valid and must stay unchanged.

`crates/server/tests/domain/migration_history_test.rs` locks specific historical filenames and SQL bodies that must not change.

## Name Validation

`scripts/lib/check-migration-ordering.sh` is the single validator: the legacy files are exactly
001..220, every other file is `YYYYMMDDHHMMSS_<description>.sql` with a valid UTC time, and no two
files share a version. CI runs it on every PR and on every push to `main` (job
`Migration Immutability`), so a duplicate that two parallel merges create shows up on the next
main run. `just pre-push` and `just pre-pr` run it too; `scripts/test-migration-ordering.sh`
covers it.

## Execution

- **Framework:** SQLx `migrate!()` macro (see `crates/server/src/setup/storage_init.rs`)
- **Auto-apply:** on startup, into the embedded database in DEV_MODE too (skippable via `--no-migrations` outside DEV_MODE)
- **Tracking table:** `_sqlx_migrations` (version, description, checksum, installed_on)
- **Build integration:** `crates/server/build.rs` watches the migrations directory so binaries rebuild when migrations change

## Conflict Resolution

Two branches no longer compete for a number. If the validator ever reports a shared version, give
the newer file a fresh timestamp before it merges; once merged, follow Immutability and add a new
migration instead of renaming.
