-- Workspace Volume sources.
--
-- Source-backed volumes expose the same volume/file surface as ordinary
-- volumes, but their contents are populated by a provider sync job and are
-- immutable to users. Credentials stay in user/identity connections; only
-- non-secret repository coordinates live on the volume row.

ALTER TABLE volumes
    ADD COLUMN source_type VARCHAR(32) NOT NULL DEFAULT 'manual'
        CHECK (source_type IN ('manual', 'github', 'git')),
    ADD COLUMN source_config JSONB NOT NULL DEFAULT '{}'::jsonb,
    ADD COLUMN is_readonly BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN sync_status VARCHAR(32) NOT NULL DEFAULT 'idle'
        CHECK (sync_status IN ('idle', 'pending', 'syncing', 'synced', 'failed')),
    ADD COLUMN last_synced_at TIMESTAMPTZ,
    ADD COLUMN last_sync_error TEXT;

CREATE INDEX idx_volumes_source_type ON volumes(org_id, source_type);
CREATE INDEX idx_volumes_sync_status ON volumes(org_id, sync_status)
    WHERE source_type != 'manual';

ALTER TABLE volumes
    ADD CONSTRAINT volumes_manual_source_shape CHECK (
        source_type != 'manual'
        OR (
            source_config = '{}'::jsonb
            AND is_readonly = FALSE
            AND sync_status = 'idle'
            AND last_synced_at IS NULL
            AND last_sync_error IS NULL
        )
    ),
    ADD CONSTRAINT volumes_source_readonly CHECK (
        source_type = 'manual' OR is_readonly = TRUE
    );
