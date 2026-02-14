-- Add org_id column to images table for multi-tenant isolation
ALTER TABLE images ADD COLUMN org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1;
CREATE INDEX idx_images_org_id ON images(org_id);
