-- Per-organization opt-in for API-visible feature flags.
CREATE TABLE org_feature_flags (
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    flag_name TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (org_id, flag_name)
);

CREATE INDEX idx_org_feature_flags_org_id ON org_feature_flags (org_id);
