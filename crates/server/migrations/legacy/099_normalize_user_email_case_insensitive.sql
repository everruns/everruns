-- EVE-704: Treat user email as a case-insensitive identity.
--
-- Previously `users.email` was stored verbatim and guarded by a case-sensitive
-- unique index (`idx_users_email`), so `John@x.com` and `john@x.com` were two
-- distinct accounts for the same mailbox and OAuth linking / login / recovery
-- could silently target the wrong one. The application now canonicalizes email
-- (trim + lowercase) at the storage trust boundary; this migration aligns
-- existing data and replaces the index with a case-insensitive one.

-- 1. Reject pre-existing case-insensitive duplicate accounts. Auto-merging is
--    unsafe here (ambiguous winner, silent loss of credentials / org
--    membership / owned data), so fail loudly and require an operator to
--    reconcile the accounts before this migration can apply. A fresh database
--    has none, so this is a no-op in normal deployments.
DO $$
DECLARE
    dup_groups INTEGER;
    dup_sample TEXT;
BEGIN
    SELECT COUNT(*), string_agg(DISTINCT canonical_email, ', ')
    INTO dup_groups, dup_sample
    FROM (
        SELECT lower(btrim(email)) AS canonical_email
        FROM users
        GROUP BY lower(btrim(email))
        HAVING COUNT(*) > 1
    ) d;

    IF dup_groups > 0 THEN
        RAISE EXCEPTION
            'EVE-704: % email address(es) map to more than one account when compared case-insensitively (e.g. %). Merge or remove the duplicate user rows, then re-run this migration.',
            dup_groups, left(dup_sample, 200);
    END IF;
END;
$$;

-- 2. Canonicalize stored emails so the persisted value matches what the
--    application now writes and looks up by. Safe after the duplicate check
--    above guarantees no collision can result.
UPDATE users
SET email = lower(btrim(email))
WHERE email <> lower(btrim(email));

-- 3. Replace the case-sensitive unique index with a case-insensitive one so the
--    database enforces the "one account per mailbox" invariant regardless of
--    casing, even against any future write path that bypasses the application
--    normalization.
DROP INDEX IF EXISTS idx_users_email;
CREATE UNIQUE INDEX idx_users_email ON users (lower(email));
