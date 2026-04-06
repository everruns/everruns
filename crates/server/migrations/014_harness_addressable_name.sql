-- Migration: Harness addressable name
--
-- Makes harness `name` the URL/CLI-friendly addressable identifier (slug).
-- Adds `display_name` for the human-readable label.
-- Migrates existing data: current name → display_name, slugified name → name.

-- Step 1: Add display_name column, copy existing name values
ALTER TABLE harnesses ADD COLUMN display_name VARCHAR(255);
UPDATE harnesses SET display_name = name;
ALTER TABLE harnesses ALTER COLUMN display_name SET NOT NULL;

-- Step 2: Convert name to slug format (lowercase, spaces/underscores → hyphens,
-- strip non-alphanumeric-hyphen, collapse consecutive hyphens, trim hyphens)
UPDATE harnesses SET name = TRIM(BOTH '-' FROM
    regexp_replace(
        regexp_replace(
            regexp_replace(
                LOWER(name),
                '[^a-z0-9-]', '-', 'g'
            ),
            '-+', '-', 'g'
        ),
        '^-|-$', '', 'g'
    )
);

-- Step 3: Handle duplicates within same org by appending a suffix.
-- After slugification, two harnesses could collide (unlikely but safe).
WITH dupes AS (
    SELECT id, org_id, name,
           ROW_NUMBER() OVER (PARTITION BY org_id, name ORDER BY created_at) AS rn
    FROM harnesses
)
UPDATE harnesses h
SET name = h.name || '-' || d.rn
FROM dupes d
WHERE h.id = d.id AND d.rn > 1;

-- Step 4: Add unique constraint per org (only for non-deleted harnesses)
CREATE UNIQUE INDEX idx_harnesses_org_name ON harnesses (org_id, name) WHERE status != 'deleted';
