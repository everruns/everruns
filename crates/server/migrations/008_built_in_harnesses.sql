-- Built-in harnesses: readonly, system-managed harnesses provisioned per-org
--
-- Adds is_built_in flag to harnesses table. Built-in harnesses are:
-- - Created automatically during org initialization
-- - Readonly (cannot be updated or deleted via API)
-- - Updated by the system during reconciliation (upgrade built-in definitions)
-- - Copyable by users who need a mutable version

ALTER TABLE harnesses ADD COLUMN is_built_in BOOLEAN NOT NULL DEFAULT FALSE;

-- Mark existing seed harnesses as built-in
UPDATE harnesses SET is_built_in = TRUE WHERE id IN (
    '01933b5a-0000-7000-8000-000000000601',  -- Base
    '01933b5a-0000-7000-8000-000000000602',  -- Generic
    '01933b5a-0000-7000-8000-000000000603'   -- Platform Chat
);
