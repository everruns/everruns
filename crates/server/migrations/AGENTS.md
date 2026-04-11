## Migrations

See [`specs/migrations.md`](../../../specs/migrations.md) for the full specification.

Key rules:

1. **Naming:** `NNN_<feature_name>.sql` — next sequential number, descriptive name
2. **After rebase:** check for duplicate numbers, renumber to next available
3. **Release squash:** before/during release, squash feature migrations into a single `NNN_vX.Y.Z.sql`
4. **Ordering:** numbers must be strictly sequential (no gaps, no duplicates) — validated by `just pre-push` and `/ship`
