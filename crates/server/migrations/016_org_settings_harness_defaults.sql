ALTER TABLE organization_settings
ADD COLUMN default_harness_id UUID REFERENCES harnesses(id) ON DELETE SET NULL,
ADD COLUMN base_harness_id UUID REFERENCES harnesses(id) ON DELETE SET NULL;

UPDATE organization_settings os
SET default_harness_id = h.id
FROM harnesses h
WHERE os.default_harness_id IS NULL
  AND h.org_id = os.org_id
  AND h.is_built_in = TRUE
  AND h.name = 'Generic';

UPDATE organization_settings os
SET base_harness_id = h.id
FROM harnesses h
WHERE os.base_harness_id IS NULL
  AND h.org_id = os.org_id
  AND h.is_built_in = TRUE
  AND h.name = 'Base';
