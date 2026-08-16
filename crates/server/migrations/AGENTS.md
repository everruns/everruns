## Migrations

See [`knowledge/operations/migrations.md`](../../../knowledge/operations/migrations.md) for the full specification.

Key rules:

1. **Naming:** `NNN_<feature_name>.sql`, next sequential number, descriptive name
2. **After rebase:** check for duplicate numbers, renumber to next available
3. **Release prep:** do not squash or rename existing migrations for a release; keep authored filenames and verify ordering
4. **Ordering:** numbers must be strictly sequential (no gaps, no duplicates), validated by `scripts/lib/check-migration-ordering.sh`, invoked by `just pre-push`, `just pre-pr`, and `/ship` (after rebase and again immediately before merge)
5. **Immutability:** once a migration SQL file is merged to `main`, never modify, delete, or rename it. Fixes must land as a new sequential migration. CI and `just pre-push` reject modified/deleted/renamed existing migration files.
