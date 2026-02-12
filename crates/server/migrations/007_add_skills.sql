-- Skills registry tables
-- Stores Agent Skills (agentskills.io format) for discovery and activation

CREATE TABLE skills (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    public_id TEXT NOT NULL,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) DEFAULT 1,
    name VARCHAR(64) NOT NULL,
    description VARCHAR(1024) NOT NULL,
    license TEXT,
    compatibility VARCHAR(500),
    metadata JSONB NOT NULL DEFAULT '{}'::jsonb,
    allowed_tools TEXT,
    instructions TEXT NOT NULL,
    source_type VARCHAR(20) NOT NULL DEFAULT 'markdown'
        CHECK (source_type IN ('markdown', 'archive')),
    archive_data BYTEA,
    status VARCHAR(50) NOT NULL DEFAULT 'active'
        CHECK (status IN ('active', 'disabled')),
    version VARCHAR(50) NOT NULL DEFAULT '1.0',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    CONSTRAINT skills_public_id_format CHECK (public_id ~ '^skill_[0-9a-f]{32}$')
);

CREATE UNIQUE INDEX idx_skills_org_public_id ON skills(org_id, public_id);
CREATE UNIQUE INDEX idx_skills_org_name ON skills(org_id, name);
CREATE INDEX idx_skills_status ON skills(status);
CREATE INDEX idx_skills_org_id ON skills(org_id);

CREATE TRIGGER update_skills_updated_at BEFORE UPDATE ON skills
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- Extracted files from archive-based skills
-- Stored individually for fast reads and VFS mounting (no ZIP extraction at runtime)
CREATE TABLE skill_files (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    skill_id UUID NOT NULL REFERENCES skills(id) ON DELETE CASCADE,
    path VARCHAR(500) NOT NULL,
    content TEXT,
    content_binary BYTEA,
    is_binary BOOLEAN NOT NULL DEFAULT FALSE,
    size_bytes BIGINT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(skill_id, path)
);

CREATE INDEX idx_skill_files_skill_id ON skill_files(skill_id);
