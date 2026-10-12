-- One org-scoped runtime account and credential store for consumers and services.
ALTER TABLE agent_identities RENAME TO virtual_users;
ALTER TABLE virtual_users ADD COLUMN usage TEXT NOT NULL DEFAULT 'service'
    CHECK (usage IN ('end_user', 'service'));
ALTER TABLE virtual_users ADD CONSTRAINT virtual_users_org_id_id_unique UNIQUE (org_id, id);
ALTER TABLE agent_identity_connections RENAME TO virtual_user_connections;
ALTER TABLE virtual_user_connections RENAME COLUMN agent_identity_id TO virtual_user_id;
ALTER TABLE agents RENAME COLUMN agent_identity_id TO virtual_user_id;
ALTER TABLE sessions RENAME COLUMN agent_identity_id TO virtual_user_id;
ALTER TABLE apps RENAME COLUMN agent_identity_id TO virtual_user_id;
ALTER TABLE agent_endpoints RENAME COLUMN agent_identity_id TO virtual_user_id;
ALTER TABLE agent_triggers RENAME COLUMN execution_agent_identity_id TO execution_virtual_user_id;
ALTER TABLE principals DROP CONSTRAINT principals_kind_check;
UPDATE principals SET kind='virtual_user' WHERE kind='agent_identity';
ALTER TABLE principals ADD CONSTRAINT principals_kind_check CHECK (kind IN ('user','virtual_user','system'));

CREATE TABLE virtual_user_bindings (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    virtual_user_id UUID NOT NULL,
    status TEXT NOT NULL DEFAULT 'active' CHECK(status IN ('active','revoked')),
    provider TEXT NOT NULL,
    realm TEXT NOT NULL,
    subject TEXT NOT NULL,
    management_user_id UUID REFERENCES users(id) ON DELETE CASCADE,
    UNIQUE(org_id,provider,realm,subject),
    FOREIGN KEY(org_id,virtual_user_id) REFERENCES virtual_users(org_id,id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX virtual_user_default_binding ON virtual_user_bindings(org_id,management_user_id)
    WHERE management_user_id IS NOT NULL;
-- Stable mapping is stored explicitly. Never infer runtime subjects from owner lineage.
INSERT INTO virtual_users(id,org_id,usage,name,avatar_url)
SELECT md5('everruns:default-virtual-user:'||m.org_id||':'||u.id)::uuid,m.org_id,'end_user',u.name,u.avatar_url
FROM organization_members m JOIN users u ON u.id=m.user_id;
INSERT INTO virtual_user_bindings(org_id,virtual_user_id,provider,realm,subject,management_user_id)
SELECT m.org_id,md5('everruns:default-virtual-user:'||m.org_id||':'||m.user_id)::uuid,
       'everruns',m.org_id::text,m.user_id::text,m.user_id FROM organization_members m;
INSERT INTO principals(public_id,org_id,kind,subject_id,parent_principal_id,resolved_user_id,metadata)
SELECT 'principal_'||replace(gen_random_uuid()::text,'-',''),b.org_id,'virtual_user',b.virtual_user_id,p.id,b.management_user_id,
       jsonb_build_object('source','virtual_user','name',v.name,'avatar_url',v.avatar_url)
FROM virtual_user_bindings b JOIN virtual_users v ON v.id=b.virtual_user_id
JOIN principals p ON p.org_id=b.org_id AND p.kind='user' AND p.subject_id=b.management_user_id;
-- Preserve chat URLs, participant IDs, pins, archived starter rows and historical events.
UPDATE sessions s SET owner_principal_id=vp.id
FROM principals up JOIN virtual_user_bindings b ON b.org_id=up.org_id AND b.management_user_id=up.subject_id
JOIN principals vp ON vp.org_id=b.org_id AND vp.kind='virtual_user' AND vp.subject_id=b.virtual_user_id
WHERE s.org_id=up.org_id AND s.owner_principal_id=up.id AND up.kind='user';
UPDATE session_participants sp SET principal_id=vp.id
FROM principals up JOIN virtual_user_bindings b ON b.org_id=up.org_id AND b.management_user_id=up.subject_id
JOIN principals vp ON vp.org_id=b.org_id AND vp.kind='virtual_user' AND vp.subject_id=b.virtual_user_id
WHERE sp.org_id=up.org_id AND sp.principal_id=up.id AND up.kind='user';
-- External actor principals retain their durable IDs and historical attribution.
INSERT INTO virtual_users(id,org_id,usage,name,status)
SELECT subject_id,org_id,'end_user',COALESCE(metadata->>'name','External user'),status
FROM principals WHERE kind='user' AND resolved_user_id IS NULL AND metadata->>'source'='external_actor'
ON CONFLICT(id) DO NOTHING;
UPDATE principals SET kind='virtual_user' WHERE kind='user' AND resolved_user_id IS NULL AND metadata->>'source'='external_actor';

-- Pending rows are a cutover queue, never read by execution. Multi-org grants require
-- an explicit destination; do not clone credentials into every membership.
ALTER TABLE user_connections RENAME TO pending_user_connections;
INSERT INTO virtual_user_connections(id,virtual_user_id,provider,connection_type,provider_user_id,provider_username,
 access_token_encrypted,refresh_token_encrypted,scopes,expires_at,installation_id,provider_metadata,created_at,updated_at)
SELECT c.id,b.virtual_user_id,c.provider,c.connection_type,c.provider_user_id,c.provider_username,
 c.access_token_encrypted,c.refresh_token_encrypted,c.scopes,c.expires_at,c.installation_id,c.provider_metadata,c.created_at,c.updated_at
FROM pending_user_connections c JOIN virtual_user_bindings b ON b.management_user_id=c.user_id
WHERE (SELECT count(*) FROM organization_members m WHERE m.user_id=c.user_id)=1;
DELETE FROM pending_user_connections c WHERE EXISTS(SELECT 1 FROM virtual_user_connections v WHERE v.id=c.id);
CREATE TABLE virtual_user_preferences (
    id UUID PRIMARY KEY DEFAULT uuidv7(), virtual_user_id UUID NOT NULL REFERENCES virtual_users(id) ON DELETE CASCADE,
    key VARCHAR(255) NOT NULL, value TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(virtual_user_id,key)
);
CREATE TABLE runtime_invocations (
    input_message_id UUID PRIMARY KEY,
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    virtual_user_id UUID,
    management_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    responder_agent_id UUID REFERENCES agents(id) ON DELETE SET NULL,
    FOREIGN KEY(org_id,virtual_user_id) REFERENCES virtual_users(org_id,id) ON DELETE RESTRICT
);

ALTER TABLE session_secrets ADD COLUMN virtual_user_id UUID REFERENCES virtual_users(id) ON DELETE CASCADE;
-- Legacy grants have no proved caller binding; they require reconnection.
ALTER TABLE leased_resources DROP CONSTRAINT leased_resources_owner_user_id_fkey;
ALTER TABLE leased_resources ADD COLUMN pending_connection_id UUID REFERENCES pending_user_connections(id) ON DELETE RESTRICT;
UPDATE leased_resources l SET pending_connection_id=c.id,metadata=l.metadata||'{"connection_migration_pending":true}'::jsonb FROM pending_user_connections c
WHERE c.user_id=l.owner_user_id AND c.provider=l.provider;

-- The lease organization proves its runtime owner even when a grant requires a
-- separate destination choice. Never move another organization's cleanup authority.
UPDATE leased_resources l SET owner_user_id=b.virtual_user_id
FROM virtual_user_bindings b
WHERE l.owner_user_id=b.management_user_id AND l.org_id=b.org_id;
UPDATE leased_resources SET owner_user_id=NULL,metadata=metadata||'{"connection_migration_pending":true}'::jsonb
WHERE owner_user_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM virtual_users v WHERE v.id=owner_user_id AND v.org_id=leased_resources.org_id);
ALTER TABLE leased_resources ADD CONSTRAINT leased_resources_owner_virtual_user_fk
    FOREIGN KEY(org_id,owner_user_id) REFERENCES virtual_users(org_id,id) ON DELETE SET NULL(owner_user_id);

ALTER TABLE pinned_sessions DROP CONSTRAINT pinned_sessions_user_id_fkey;
UPDATE pinned_sessions p SET user_id=b.virtual_user_id
FROM virtual_user_bindings b WHERE b.org_id=p.org_id AND b.management_user_id=p.user_id;
DELETE FROM pinned_sessions WHERE NOT EXISTS(SELECT 1 FROM virtual_users v WHERE v.id=user_id);
ALTER TABLE pinned_sessions ADD CONSTRAINT pinned_sessions_virtual_user_fkey FOREIGN KEY(user_id) REFERENCES virtual_users(id) ON DELETE CASCADE;
-- Verified realms keep historical external principals. Unqualified legacy actors
-- remain unbound until a verified identity can establish an unambiguous mapping.
INSERT INTO virtual_user_bindings(org_id,virtual_user_id,provider,realm,subject)
SELECT p.org_id,p.subject_id,p.metadata->>'external_source',p.metadata->'external_actor_metadata'->>'team_id',p.metadata->>'external_actor_id'
FROM principals p WHERE p.kind='virtual_user' AND p.metadata->>'source'='external_actor'
 AND COALESCE(p.metadata->'external_actor_metadata'->>'team_id','')<>''
ON CONFLICT(org_id,provider,realm,subject) DO NOTHING;

-- Runtime resource references cannot cross organizations, even through an internal writer.
ALTER TABLE agents ADD CONSTRAINT agents_service_virtual_user_org_fk
    FOREIGN KEY(org_id,virtual_user_id) REFERENCES virtual_users(org_id,id) ON DELETE SET NULL(virtual_user_id);
ALTER TABLE sessions ADD CONSTRAINT sessions_virtual_user_org_fk
    FOREIGN KEY(org_id,virtual_user_id) REFERENCES virtual_users(org_id,id) ON DELETE SET NULL(virtual_user_id);
ALTER TABLE apps ADD CONSTRAINT apps_virtual_user_org_fk
    FOREIGN KEY(org_id,virtual_user_id) REFERENCES virtual_users(org_id,id) ON DELETE SET NULL(virtual_user_id);
ALTER TABLE agent_triggers ADD CONSTRAINT triggers_virtual_user_org_fk
    FOREIGN KEY(org_id,execution_virtual_user_id) REFERENCES virtual_users(org_id,id) ON DELETE SET NULL(execution_virtual_user_id);
CREATE FUNCTION enforce_agent_service_virtual_user() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.virtual_user_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM virtual_users WHERE id=NEW.virtual_user_id AND org_id=NEW.org_id AND usage='service' AND status='active'
    ) THEN
        RAISE EXCEPTION 'Agent requires an active service virtual user in its organization';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER agent_service_virtual_user_guard BEFORE INSERT OR UPDATE OF virtual_user_id ON agents
    FOR EACH ROW EXECUTE FUNCTION enforce_agent_service_virtual_user();

-- Endpoint tenancy is inherited from its agent, rather than stored on the endpoint.
CREATE FUNCTION enforce_endpoint_virtual_user_org() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.virtual_user_id IS NOT NULL AND NOT EXISTS (
        SELECT 1 FROM virtual_users v JOIN agents a ON a.org_id=v.org_id
        WHERE v.id=NEW.virtual_user_id AND a.id=NEW.agent_id
    ) THEN
        RAISE EXCEPTION 'Endpoint virtual user must belong to the agent organization';
    END IF;
    RETURN NEW;
END;
$$;
CREATE TRIGGER endpoint_virtual_user_org_guard BEFORE INSERT OR UPDATE OF virtual_user_id,agent_id ON agent_endpoints
    FOR EACH ROW EXECUTE FUNCTION enforce_endpoint_virtual_user_org();

-- Setup cookies carry a hash-bound payload; this ledger enforces server-side
-- expiry and a single callback across replicas without storing provider secrets.
CREATE TABLE connection_setup_states (
    state TEXT PRIMARY KEY,
    provider TEXT NOT NULL,
    payload_hash BYTEA NOT NULL,
    expires_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX connection_setup_states_expiry ON connection_setup_states(expires_at);

-- Newly introduced GitHub Apps use the same canonical credential owner.
ALTER TABLE github_apps RENAME COLUMN agent_identity_id TO virtual_user_id;
ALTER TABLE github_apps ADD CONSTRAINT github_apps_virtual_user_org_fk
    FOREIGN KEY(org_id,virtual_user_id) REFERENCES virtual_users(org_id,id) ON DELETE CASCADE;

ALTER TABLE payment_policies DROP CONSTRAINT payment_policies_subject_type_check;
UPDATE payment_policies SET subject_type='virtual_user' WHERE subject_type='agent_identity';
ALTER TABLE payment_policies ADD CONSTRAINT payment_policies_subject_type_check
    CHECK(subject_type IN ('user','virtual_user','agent','agent_endpoint','session','org'));

ALTER TABLE payment_accounts DROP CONSTRAINT payment_accounts_owner_type_check;
UPDATE payment_accounts SET owner_type='virtual_user' WHERE owner_type='agent_identity';
ALTER TABLE payment_accounts ADD CONSTRAINT payment_accounts_owner_type_check
    CHECK(owner_type IN ('user','virtual_user','organization'));
