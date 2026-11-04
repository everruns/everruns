-- Repair discovered models that a pre-#5095 model sync marked "No longer listed".
--
-- That sync stamped each seen model's last_seen_at with the sync start time,
-- left new models' last_seen_at NULL, and stamped the provider's
-- last_synced_at with a later clock reading at the end. Staleness is read as
-- last_seen_at < last_synced_at, so every model the provider listed read as
-- stale. #5095 fixed the sync but relied on the next periodic sync to heal the
-- rows, and deployments that disable the periodic sync
-- (MODEL_SYNC_INTERVAL_HOURS=0) kept them stale: chat model pickers showed
-- "No configured chat models" and flagged the org default as unavailable.
--
-- Decision: a row carries that signature when its last_seen_at (or, for a row
-- the sync created, its created_at) falls within the same sync run, i.e. a few
-- minutes before the provider's last_synced_at. A model the provider genuinely
-- stopped listing was last seen by an earlier sync and stays stale.
UPDATE models m
SET last_seen_at = p.last_synced_at
FROM providers p
WHERE p.org_id = m.org_id
  AND p.id = m.provider_id
  AND m.source = 'discovered'
  AND p.last_synced_at IS NOT NULL
  AND (
      (m.last_seen_at IS NOT NULL
       AND m.last_seen_at < p.last_synced_at
       AND m.last_seen_at >= p.last_synced_at - interval '15 minutes')
      OR (m.last_seen_at IS NULL
          AND m.created_at >= p.last_synced_at - interval '15 minutes')
  );
