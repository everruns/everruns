-- Data plane v2's Raft groups are shards, not cells (agentd rename). Data
-- plane v2 has no real use yet, so the names change in place, keeping rows.
ALTER TABLE dp_grants RENAME COLUMN cell TO shard;

ALTER TABLE monitoring_cell_minutes RENAME TO monitoring_shard_minutes;
ALTER TABLE monitoring_shard_minutes RENAME COLUMN cell TO shard;
ALTER INDEX idx_monitoring_cell_minutes_minute RENAME TO idx_monitoring_shard_minutes_minute;
ALTER INDEX monitoring_cell_minutes_pkey RENAME TO monitoring_shard_minutes_pkey;
