-- Migration: Harness addressable name
--
-- Makes harness `name` the URL/CLI-friendly addressable identifier (slug).
-- Adds `display_name` for the human-readable label.
-- Migrates existing data: current name → display_name, slugified name → name.

-- Step 1: Add display_name column, copy existing name values
ALTER TABLE harnesses ADD COLUMN display_name TEXT;
UPDATE harnesses SET display_name = name;
ALTER TABLE harnesses ALTER COLUMN display_name SET NOT NULL;

-- Step 2: Convert name to slug format (lowercase, spaces/underscores → hyphens,
-- strip non-alphanumeric-hyphen, collapse consecutive hyphens, trim hyphens).
-- Fall back to 'harness-<short-id>' if slugification yields empty string
-- (e.g. names made only of symbols/emoji).
UPDATE harnesses
SET name = CASE
    WHEN TRIM(BOTH '-' FROM
        regexp_replace(
            regexp_replace(
                regexp_replace(LOWER(name), '[^a-z0-9-]', '-', 'g'),
                '-+', '-', 'g'
            ),
            '^-|-$', '', 'g'
        )
    ) = '' THEN 'harness-' || LEFT(REPLACE(id::text, '-', ''), 8)
    ELSE TRIM(BOTH '-' FROM
        regexp_replace(
            regexp_replace(
                regexp_replace(LOWER(name), '[^a-z0-9-]', '-', 'g'),
                '-+', '-', 'g'
            ),
            '^-|-$', '', 'g'
        )
    )
END;

-- Step 3: Handle duplicates within same org by appending a suffix.
-- Choose suffixes that don't collide with any existing name in the org.
WITH dupes AS (
    SELECT id, org_id, name,
           ROW_NUMBER() OVER (PARTITION BY org_id, name ORDER BY created_at, id) AS rn
    FROM harnesses
),
max_existing AS (
    SELECT d.org_id, d.name,
           COALESCE(MAX(
               CASE
                   WHEN h2.name ~ ('^' || d.name || '-[0-9]+$')
                   THEN SUBSTRING(h2.name FROM '-([0-9]+)$')::integer
                   ELSE 0
               END
           ), 0) AS max_suffix
    FROM (SELECT DISTINCT org_id, name FROM dupes WHERE rn > 1) d
    JOIN harnesses h2 ON h2.org_id = d.org_id
    GROUP BY d.org_id, d.name
)
UPDATE harnesses h
SET name = d.name || '-' || (me.max_suffix + d.rn)
FROM dupes d
JOIN max_existing me ON me.org_id = d.org_id AND me.name = d.name
WHERE h.id = d.id AND d.rn > 1;

-- Step 4: Add unique constraint per org (only for non-deleted harnesses)
CREATE UNIQUE INDEX idx_harnesses_org_name ON harnesses (org_id, name) WHERE status != 'deleted';
