## Migrations

See [`specs/migrations.md`](../../../specs/migrations.md) for the full specification.

Key rules:

1. **Naming:** `NNN_<feature_name>.sql` — next sequential number, descriptive name
2. **After rebase:** check for duplicate numbers, renumber to next available
3. **Release prep:** do not squash or rename existing migrations for a release; keep authored filenames and verify ordering
4. **Ordering:** numbers must be strictly sequential (no gaps, no duplicates) — validated by `just pre-push` and `/ship`
