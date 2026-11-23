-- An org placed on a data plane that never ran a session there can go back
-- to the platform's own workers (v1): `everruns-dataplane to-v1`. The row
-- stays (records are never purged) with the time it went back; every lookup
-- that answers "is this org on a data plane" skips such a row, and enabling
-- the org again clears it.
ALTER TABLE dp_orgs ADD COLUMN returned_at TIMESTAMPTZ;
