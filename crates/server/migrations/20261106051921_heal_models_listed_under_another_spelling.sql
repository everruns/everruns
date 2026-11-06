-- Heal discovered models that read "no longer listed" only because the
-- provider lists them under its own spelling.
--
-- A model row moved from one provider to another keeps the id it had: a
-- GPT-6.1 Sol row moved from the OpenAI provider to OpenRouter stays
-- `gpt-6.1-sol` while OpenRouter lists `openai/gpt-6.1-sol`. The model sync
-- matched rows by exact id, so it never stamped such a row as seen, and the
-- model pickers and the "Agent's model is unavailable" health issue reported
-- it as no longer listed while the provider still served it. The sync now
-- matches by registry identity too (`listed_under_any_spelling` in
-- domains/models/sync.rs), but deployments that never sync on their own
-- (MODEL_SYNC_INTERVAL_HOURS=0) would keep these rows stale.
--
-- Decision: repair only the case the stored rows prove: the row's registry
-- identity (`vendor/canonical`) is, character for character, the id of a
-- sibling row on the same provider that the provider's last sync listed.
-- Anything else heals at the next sync.
UPDATE models m
SET last_seen_at = p.last_synced_at
FROM providers p
WHERE p.org_id = m.org_id
  AND p.id = m.provider_id
  AND m.source = 'discovered'
  AND p.last_synced_at IS NOT NULL
  AND (m.last_seen_at IS NULL OR m.last_seen_at < p.last_synced_at)
  AND EXISTS (
      SELECT 1
      FROM models listed
      WHERE listed.org_id = m.org_id
        AND listed.provider_id = m.provider_id
        AND listed.id <> m.id
        AND listed.last_seen_at >= p.last_synced_at
        AND listed.model_id = m.provider_metadata -> 'everruns_catalog' ->> 'profile_key'
  );
