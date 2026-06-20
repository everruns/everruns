-- Projects: a grouping layer nested inside an organization.
--
-- Direction F ("Projects on top"): an Organization is the billing/team wrapper;
-- a Project is the day-to-day work scope users switch between. Project-scoped
-- resources (agents, skills, apps, capabilities, MCP servers, memory stores,
-- sessions) belong to exactly one project. Workspace-shared resources
-- (harnesses, agent identities, models, volumes, ...) stay org-scoped.
--
-- Migration strategy mirrors the org model: every org gets exactly one
-- `default` project, and existing project-scoped rows are reparented onto it.

CREATE TABLE projects (
    project_id BIGSERIAL PRIMARY KEY,
    -- Dual-ID pattern: internal BIGINT (project_id) + external public_id.
    public_id TEXT UNIQUE NOT NULL,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    description TEXT,
    -- Exactly one default project per org; the migration target for existing
    -- resources and the fallback scope when no project is selected.
    is_default BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    CONSTRAINT projects_public_id_format
        CHECK (public_id ~ '^proj_[0-9a-f]{32}$')
);

CREATE INDEX idx_projects_public_id ON projects(public_id);
CREATE INDEX idx_projects_org_id ON projects(org_id);
-- Project names are unique per org (case-insensitive), like a folder name.
CREATE UNIQUE INDEX idx_projects_org_name ON projects(org_id, lower(name));
-- At most one default project per org.
CREATE UNIQUE INDEX idx_projects_org_default ON projects(org_id) WHERE is_default;

CREATE TRIGGER update_projects_updated_at BEFORE UPDATE ON projects
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- Seed the default project for the seeded default org (project_id = 1), with a
-- well-known public_id mirroring DEFAULT_ORG_PUBLIC_ID. Required so FKs and the
-- DEFAULT_PROJECT_ID fallback resolve on a fresh database.
INSERT INTO projects (project_id, public_id, org_id, name, is_default)
VALUES (1, 'proj_00000000000000000000000000000001', 1, 'Default', TRUE);

SELECT setval('projects_project_id_seq', 1, true);

-- Backfill a default project for every other existing org. public_id is a
-- random 32-hex value (md5 avoids a pgcrypto dependency).
INSERT INTO projects (public_id, org_id, name, is_default)
SELECT 'proj_' || md5(random()::text || clock_timestamp()::text || o.org_id::text),
       o.org_id, 'Default', TRUE
FROM organizations o
WHERE o.org_id <> 1;

-- ============================================
-- Reparent project-scoped resources onto each org's default project.
-- ============================================
-- Pattern per table:
--   1. ADD COLUMN project_id (nullable, no default yet)
--   2. UPDATE to the org's default project
--   3. SET NOT NULL + DEFAULT 1 (mirrors org_id DEFAULT 1 safety net)
--   4. FK + index

ALTER TABLE agents ADD COLUMN project_id BIGINT REFERENCES projects(project_id);
UPDATE agents a SET project_id = p.project_id
    FROM projects p WHERE p.org_id = a.org_id AND p.is_default;
ALTER TABLE agents ALTER COLUMN project_id SET NOT NULL;
ALTER TABLE agents ALTER COLUMN project_id SET DEFAULT 1;
CREATE INDEX idx_agents_project_id ON agents(project_id);

ALTER TABLE skills ADD COLUMN project_id BIGINT REFERENCES projects(project_id);
UPDATE skills s SET project_id = p.project_id
    FROM projects p WHERE p.org_id = s.org_id AND p.is_default;
ALTER TABLE skills ALTER COLUMN project_id SET NOT NULL;
ALTER TABLE skills ALTER COLUMN project_id SET DEFAULT 1;
CREATE INDEX idx_skills_project_id ON skills(project_id);

ALTER TABLE apps ADD COLUMN project_id BIGINT REFERENCES projects(project_id);
UPDATE apps a SET project_id = p.project_id
    FROM projects p WHERE p.org_id = a.org_id AND p.is_default;
ALTER TABLE apps ALTER COLUMN project_id SET NOT NULL;
ALTER TABLE apps ALTER COLUMN project_id SET DEFAULT 1;
CREATE INDEX idx_apps_project_id ON apps(project_id);

ALTER TABLE declarative_capabilities ADD COLUMN project_id BIGINT REFERENCES projects(project_id);
UPDATE declarative_capabilities c SET project_id = p.project_id
    FROM projects p WHERE p.org_id = c.org_id AND p.is_default;
ALTER TABLE declarative_capabilities ALTER COLUMN project_id SET NOT NULL;
ALTER TABLE declarative_capabilities ALTER COLUMN project_id SET DEFAULT 1;
CREATE INDEX idx_declarative_capabilities_project_id ON declarative_capabilities(project_id);

ALTER TABLE mcp_servers ADD COLUMN project_id BIGINT REFERENCES projects(project_id);
UPDATE mcp_servers m SET project_id = p.project_id
    FROM projects p WHERE p.org_id = m.org_id AND p.is_default;
ALTER TABLE mcp_servers ALTER COLUMN project_id SET NOT NULL;
ALTER TABLE mcp_servers ALTER COLUMN project_id SET DEFAULT 1;
CREATE INDEX idx_mcp_servers_project_id ON mcp_servers(project_id);

ALTER TABLE memory_stores ADD COLUMN project_id BIGINT REFERENCES projects(project_id);
UPDATE memory_stores m SET project_id = p.project_id
    FROM projects p WHERE p.org_id = m.org_id AND p.is_default;
ALTER TABLE memory_stores ALTER COLUMN project_id SET NOT NULL;
ALTER TABLE memory_stores ALTER COLUMN project_id SET DEFAULT 1;
CREATE INDEX idx_memory_stores_project_id ON memory_stores(project_id);

ALTER TABLE sessions ADD COLUMN project_id BIGINT REFERENCES projects(project_id);
UPDATE sessions s SET project_id = p.project_id
    FROM projects p WHERE p.org_id = s.org_id AND p.is_default;
ALTER TABLE sessions ALTER COLUMN project_id SET NOT NULL;
ALTER TABLE sessions ALTER COLUMN project_id SET DEFAULT 1;
CREATE INDEX idx_sessions_project_id ON sessions(project_id);

COMMENT ON TABLE projects IS
    'Grouping layer nested inside an organization (Direction F). Project-scoped resources belong to exactly one project.';
COMMENT ON COLUMN projects.is_default IS
    'Exactly one per org; reparent target for existing resources and fallback when no project is selected.';
