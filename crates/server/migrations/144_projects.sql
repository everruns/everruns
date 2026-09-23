-- Projects: a grouping layer nested inside an organization.
--
-- The project boundary is the Agent (knowledge/security/multitenancy.md,
-- "Projects (nested scope)"). Only rows whose project cannot be derived store
-- it: agents (the anchor) and sessions (runtime, derived from their agent).
-- Everything an agent or session owns (endpoints, triggers, versions, files,
-- tasks, ...) inherits scope through its parent. Shared registries (harnesses,
-- models, identities, skills, MCP servers, capabilities, knowledge, org memory)
-- stay org-scoped, and retired Apps are not scoped.
--
-- Every org gets exactly one `default` project and existing rows are
-- reparented onto it.

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
        CHECK (public_id ~ '^proj_[0-9a-f]{32}$'),
    -- Target of the composite (project_id, org_id) foreign keys below, so a
    -- scoped row can never point at another organization's project.
    CONSTRAINT projects_project_org_unique UNIQUE (project_id, org_id)
);

CREATE INDEX idx_projects_org_id ON projects(org_id);
-- Project names are unique per org (case-insensitive), like a folder name.
CREATE UNIQUE INDEX idx_projects_org_name ON projects(org_id, lower(name));
-- At most one default project per org.
CREATE UNIQUE INDEX idx_projects_org_default ON projects(org_id) WHERE is_default;

CREATE TRIGGER update_projects_updated_at BEFORE UPDATE ON projects
    FOR EACH ROW EXECUTE FUNCTION update_updated_at_column();

-- Seed the default project for the seeded default org (project_id = 1), with a
-- well-known public_id mirroring DEFAULT_ORG_PUBLIC_ID.
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

-- Every org created from now on gets its default project in the same
-- statement, so the invariant holds for every creation path, not only the API.
CREATE FUNCTION organizations_create_default_project() RETURNS trigger AS $$
BEGIN
    INSERT INTO projects (public_id, org_id, name, is_default)
    VALUES (
        'proj_' || md5(random()::text || clock_timestamp()::text || NEW.org_id::text),
        NEW.org_id, 'Default', TRUE
    )
    ON CONFLICT DO NOTHING;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER organizations_create_default_project_after_insert
    AFTER INSERT ON organizations
    FOR EACH ROW EXECUTE FUNCTION organizations_create_default_project();

-- ============================================
-- Agents: the anchor of project scope.
-- ============================================

ALTER TABLE agents ADD COLUMN project_id BIGINT;
UPDATE agents a SET project_id = p.project_id
    FROM projects p WHERE p.org_id = a.org_id AND p.is_default;
ALTER TABLE agents ALTER COLUMN project_id SET NOT NULL;
ALTER TABLE agents ADD CONSTRAINT agents_project_fk
    FOREIGN KEY (project_id, org_id) REFERENCES projects(project_id, org_id);
CREATE INDEX idx_agents_org_project ON agents(org_id, project_id);
-- Agent names are unique per project, not per org: a name in another project
-- must not collide (or reveal that it exists).
DROP INDEX idx_agents_org_name;
CREATE UNIQUE INDEX idx_agents_org_project_name
    ON agents (org_id, project_id, name) WHERE status != 'deleted';

-- An insert that does not name a project lands in its org's default project.
-- Deliberately not a column DEFAULT: a constant would point every org at the
-- default org's project.
CREATE FUNCTION agents_assign_project() RETURNS trigger AS $$
BEGIN
    IF NEW.project_id IS NULL THEN
        NEW.project_id := (
            SELECT p.project_id FROM projects p
            WHERE p.org_id = NEW.org_id AND p.is_default
        );
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER agents_assign_project_before_insert BEFORE INSERT ON agents
    FOR EACH ROW EXECUTE FUNCTION agents_assign_project();

-- ============================================
-- Sessions: runtime, scoped by the agent they run.
-- ============================================

ALTER TABLE sessions ADD COLUMN project_id BIGINT;
UPDATE sessions s SET project_id = COALESCE(
    (SELECT a.project_id FROM agents a WHERE a.id = s.agent_id),
    (SELECT p.project_id FROM projects p WHERE p.org_id = s.org_id AND p.is_default)
);
ALTER TABLE sessions ALTER COLUMN project_id SET NOT NULL;
ALTER TABLE sessions ADD CONSTRAINT sessions_project_fk
    FOREIGN KEY (project_id, org_id) REFERENCES projects(project_id, org_id);
CREATE INDEX idx_sessions_org_project ON sessions(org_id, project_id);

-- A session's project follows what it runs: its agent's project, else its
-- parent session's (subagents), else the org default. Every session insert
-- path gets this without threading a project through each caller; the create
-- API reassigns an agentless session to the caller's active project.
CREATE FUNCTION sessions_assign_project() RETURNS trigger AS $$
BEGIN
    IF NEW.project_id IS NULL THEN
        NEW.project_id := COALESCE(
            (SELECT a.project_id FROM agents a WHERE a.id = NEW.agent_id),
            (SELECT s.project_id FROM sessions s WHERE s.id = NEW.parent_session_id),
            (SELECT p.project_id FROM projects p
             WHERE p.org_id = NEW.org_id AND p.is_default)
        );
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER sessions_assign_project_before_insert BEFORE INSERT ON sessions
    FOR EACH ROW EXECUTE FUNCTION sessions_assign_project();

COMMENT ON TABLE projects IS
    'Grouping of agents inside an organization. Agents and sessions store project_id; everything they own inherits it.';
COMMENT ON COLUMN projects.is_default IS
    'Exactly one per org; reparent target for existing rows and fallback when no project is selected.';
