-- Non-financial supplier receipts. Hosts decide which resources are billable
-- and price them outside the OSS domain. No content or credentials are retained.
CREATE TABLE knowledge_index_usage (
    id UUID PRIMARY KEY,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id),
    collection_id UUID NOT NULL,
    source_ref TEXT NOT NULL UNIQUE,
    written_bytes BIGINT NOT NULL CHECK (written_bytes >= 0),
    queried_bytes BIGINT NOT NULL CHECK (queried_bytes >= 0),
    returned_bytes BIGINT NOT NULL CHECK (returned_bytes >= 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX knowledge_index_usage_org_time ON knowledge_index_usage(org_id,created_at,id);
