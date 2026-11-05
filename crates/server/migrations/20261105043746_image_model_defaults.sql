-- Organization default image model, used by the image_generation capability
-- when an agent does not pick one. Mirrors decision_model_defaults.
CREATE TABLE image_model_defaults (
    org_id bigint NOT NULL PRIMARY KEY REFERENCES organizations(org_id) ON DELETE CASCADE,
    model_id uuid NOT NULL,
    FOREIGN KEY (org_id, model_id) REFERENCES models(org_id, id) ON DELETE CASCADE
);

-- The gpt_image_gen capability became the provider-neutral image_generation.
-- Rewrite stored references so existing agents, harnesses and sessions keep
-- their image tools and settings.
UPDATE agent_capabilities AS ac SET capability_id = 'image_generation'
WHERE ac.capability_id = 'gpt_image_gen'
  AND NOT EXISTS (
      SELECT 1 FROM agent_capabilities o
      WHERE o.agent_id = ac.agent_id AND o.capability_id = 'image_generation'
  );
DELETE FROM agent_capabilities WHERE capability_id = 'gpt_image_gen';

UPDATE harness_capabilities AS hc SET capability_id = 'image_generation'
WHERE hc.capability_id = 'gpt_image_gen'
  AND NOT EXISTS (
      SELECT 1 FROM harness_capabilities o
      WHERE o.harness_id = hc.harness_id AND o.capability_id = 'image_generation'
  );
DELETE FROM harness_capabilities WHERE capability_id = 'gpt_image_gen';

UPDATE sessions SET capabilities = (
    SELECT jsonb_agg(
        CASE WHEN entry->>'ref' = 'gpt_image_gen'
            THEN jsonb_set(entry, '{ref}', '"image_generation"')
            ELSE entry END
        ORDER BY position)
    FROM jsonb_array_elements(capabilities) WITH ORDINALITY AS item(entry, position)
)
WHERE capabilities @> '[{"ref": "gpt_image_gen"}]';

UPDATE payment_policies
SET allowed_capabilities = array_replace(allowed_capabilities, 'gpt_image_gen', 'image_generation')
WHERE 'gpt_image_gen' = ANY(allowed_capabilities);
