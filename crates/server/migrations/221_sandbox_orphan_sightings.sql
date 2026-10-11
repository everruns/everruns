-- Orphaned managed sandbox scan (background/sandbox_orphan_scan.rs).
--
-- The scan lists provider-side sandboxes labelled `everruns.managed=true` and
-- acts on the ones no live `sandbox_instances` row tracks, in two phases: the
-- first sighting stops the sandbox, a sighting at least a day later deletes
-- it. This table remembers the first phase between hourly runs.

CREATE TABLE sandbox_orphan_sightings (
    provider      TEXT        NOT NULL,
    external_id   TEXT        NOT NULL,
    org_id        BIGINT      NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    first_seen_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    stopped_at    TIMESTAMPTZ,
    deleted_at    TIMESTAMPTZ,
    PRIMARY KEY (provider, external_id)
);

CREATE INDEX idx_sandbox_orphan_sightings_last_seen
    ON sandbox_orphan_sightings (last_seen_at);

COMMENT ON TABLE sandbox_orphan_sightings IS
    'Provider sandboxes labelled as Everruns-managed that no live sandbox_instances row tracks. '
    'First sighting stops; a sighting 24h later deletes.';
