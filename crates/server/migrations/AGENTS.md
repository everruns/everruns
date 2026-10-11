## Migrations

See [`knowledge/operations/migrations.md`](../../../knowledge/operations/migrations.md) for the full specification.

Key rules:

1. **Naming:** `just new-migration <description>` creates `YYYYMMDDHHMMSS_<description>.sql` (UTC). Never add a `NNN_` file: 001..222 are frozen history.
2. **After rebase:** nothing to renumber. If a newer migration from main must run before yours, recreate yours so it sorts after it.
3. **Release prep:** do not squash or rename existing migrations for a release; keep authored filenames.
4. **Validation:** `scripts/lib/check-migration-ordering.sh` checks names and unique versions; CI runs it on PRs and main, `just pre-push` and `just pre-pr` locally.
5. **Immutability:** once a migration SQL file is merged to `main`, never modify, delete, or rename it. Fixes land as a new migration. CI and `just pre-push` reject modified/deleted/renamed existing migration files.
