-- scoped_resource_credentials
-- Shared ownership stays independent of provider payload and runtime identity.
CREATE UNIQUE INDEX IF NOT EXISTS resource_agents_org_id ON agents(org_id,id);
CREATE UNIQUE INDEX IF NOT EXISTS resource_sessions_org_id ON sessions(org_id,id);
CREATE UNIQUE INDEX IF NOT EXISTS resource_virtual_users_org_id ON virtual_users(org_id,id);

CREATE TABLE resource_access (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('credential_binding','connection')),
    scope TEXT NOT NULL CHECK (scope IN ('session','agent','virtual_user','organization')),
    owner_agent_id UUID,
    owner_session_id UUID,
    owner_virtual_user_id UUID,
    subject_id UUID,
    usage TEXT NOT NULL CHECK (usage IN ('user','service')),
    revision BIGINT NOT NULL DEFAULT 1 CHECK (revision > 0),
    revoked_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(org_id,id),
    FOREIGN KEY(org_id,owner_agent_id) REFERENCES agents(org_id,id) ON DELETE CASCADE,
    FOREIGN KEY(org_id,owner_session_id) REFERENCES sessions(org_id,id) ON DELETE CASCADE,
    FOREIGN KEY(org_id,owner_virtual_user_id) REFERENCES virtual_users(org_id,id) ON DELETE CASCADE,
    FOREIGN KEY(org_id,subject_id) REFERENCES virtual_users(org_id,id) ON DELETE CASCADE,
    CHECK (
      (scope='organization' AND usage='service' AND num_nonnulls(owner_agent_id,owner_session_id,owner_virtual_user_id)=0)
      OR (scope='agent' AND owner_agent_id IS NOT NULL AND num_nonnulls(owner_agent_id,owner_session_id,owner_virtual_user_id)=1 AND usage='service')
      OR (scope='session' AND owner_session_id IS NOT NULL AND num_nonnulls(owner_agent_id,owner_session_id,owner_virtual_user_id)=1)
      OR (scope='virtual_user' AND owner_virtual_user_id IS NOT NULL AND num_nonnulls(owner_agent_id,owner_session_id,owner_virtual_user_id)=1)
    ),
    CHECK (scope='session' OR subject_id IS NULL),
    CHECK (usage='service' OR scope<>'session' OR subject_id IS NOT NULL)
);
CREATE INDEX resource_access_owner_agent ON resource_access(org_id,owner_agent_id) WHERE owner_agent_id IS NOT NULL;
CREATE INDEX resource_access_owner_user ON resource_access(org_id,owner_virtual_user_id) WHERE owner_virtual_user_id IS NOT NULL;
CREATE INDEX resource_access_owner_session ON resource_access(org_id,owner_session_id) WHERE owner_session_id IS NOT NULL;

CREATE TABLE resource_grants (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    org_id BIGINT NOT NULL,
    resource_id UUID NOT NULL,
    attachment_id UUID,
    consumer_scope TEXT NOT NULL CHECK (consumer_scope IN ('session','agent','virtual_user','organization')),
    consumer_agent_id UUID,
    consumer_session_id UUID,
    consumer_virtual_user_id UUID,
    consumer_subject_id UUID,
    usage TEXT NOT NULL CHECK (usage IN ('user','service')),
    operation TEXT NOT NULL DEFAULT 'use' CHECK(operation='use'),
    constraints JSONB,
    authorizer_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
    expires_at TIMESTAMPTZ,
    revoked_at TIMESTAMPTZ,
    revision BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    FOREIGN KEY(org_id,resource_id) REFERENCES resource_access(org_id,id) ON DELETE CASCADE,
    FOREIGN KEY(org_id,consumer_agent_id) REFERENCES agents(org_id,id) ON DELETE CASCADE,
    FOREIGN KEY(org_id,consumer_session_id) REFERENCES sessions(org_id,id) ON DELETE CASCADE,
    CHECK(consumer_scope='session' OR consumer_subject_id IS NULL),
    CHECK(usage='service' OR consumer_scope<>'session' OR consumer_subject_id IS NOT NULL),
    FOREIGN KEY(org_id,consumer_subject_id) REFERENCES virtual_users(org_id,id) ON DELETE CASCADE,
    FOREIGN KEY(org_id,consumer_virtual_user_id) REFERENCES virtual_users(org_id,id) ON DELETE CASCADE,
    CHECK (
      (consumer_scope='organization' AND num_nonnulls(consumer_agent_id,consumer_session_id,consumer_virtual_user_id)=0 AND usage='service')
      OR (consumer_scope='agent' AND consumer_agent_id IS NOT NULL AND num_nonnulls(consumer_agent_id,consumer_session_id,consumer_virtual_user_id)=1 AND usage='service')
      OR (consumer_scope='session' AND consumer_session_id IS NOT NULL AND num_nonnulls(consumer_agent_id,consumer_session_id,consumer_virtual_user_id)=1)
      OR (consumer_scope='virtual_user' AND consumer_virtual_user_id IS NOT NULL AND num_nonnulls(consumer_agent_id,consumer_session_id,consumer_virtual_user_id)=1)
    )
);
CREATE INDEX resource_grants_resource ON resource_grants(org_id,resource_id) WHERE revoked_at IS NULL;

ALTER TABLE agent_mcp_secret_bindings RENAME TO credential_bindings;
ALTER TABLE credential_bindings ALTER COLUMN agent_id DROP NOT NULL;
ALTER TABLE credential_bindings ADD COLUMN expires_at TIMESTAMPTZ;
ALTER TABLE credential_bindings ADD COLUMN last_used_at TIMESTAMPTZ;
ALTER TABLE credential_bindings ADD COLUMN last_used_agent_id UUID REFERENCES agents(id) ON DELETE SET NULL;
ALTER TABLE credential_bindings ADD COLUMN owner_key TEXT;
ALTER TABLE credential_bindings ADD COLUMN source_connection_id UUID REFERENCES virtual_user_connections(id) ON DELETE CASCADE;
ALTER TABLE credential_bindings ADD COLUMN source_github_access JSONB;
UPDATE credential_bindings SET owner_key='agent:'||agent_id::text||':service:';
ALTER TABLE credential_bindings ALTER COLUMN owner_key SET NOT NULL;
CREATE UNIQUE INDEX credential_bindings_owner_mcp_target ON credential_bindings(org_id,owner_key,mcp_server_name,tool_name,parameter_name) WHERE kind='mcp_parameter';
CREATE UNIQUE INDEX credential_bindings_owner_http_name ON credential_bindings(org_id,owner_key,env_name) WHERE kind='http';

INSERT INTO resource_access(id,org_id,kind,scope,owner_agent_id,usage)
 SELECT id,org_id,'credential_binding','agent',agent_id,'service' FROM credential_bindings;
ALTER TABLE credential_bindings ADD CONSTRAINT credential_bindings_access
 FOREIGN KEY(org_id,id) REFERENCES resource_access(org_id,id) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED;

-- Existing connection ownership is canonical; retain every provider payload.
INSERT INTO resource_access(id,org_id,kind,scope,owner_virtual_user_id,usage)
 SELECT c.id,v.org_id,'connection',
 CASE WHEN c.owner_scope='organization' THEN 'organization' ELSE 'virtual_user' END,
 CASE WHEN c.owner_scope='organization' THEN NULL ELSE v.id END,
 CASE WHEN v.usage='end_user' THEN 'user' ELSE 'service' END
 FROM virtual_user_connections c JOIN virtual_users v ON v.id=c.virtual_user_id;
ALTER TABLE virtual_user_connections ADD COLUMN org_id BIGINT;
UPDATE virtual_user_connections c SET org_id=v.org_id FROM virtual_users v WHERE v.id=c.virtual_user_id;
ALTER TABLE virtual_user_connections ALTER COLUMN org_id SET NOT NULL;
CREATE UNIQUE INDEX virtual_user_connections_resource_org ON virtual_user_connections(org_id,id);
ALTER TABLE resource_grants ADD CONSTRAINT resource_grants_attachment_tenant FOREIGN KEY(org_id,attachment_id) REFERENCES virtual_user_connections(org_id,id) ON DELETE CASCADE;
ALTER TABLE virtual_user_connections ADD CONSTRAINT virtual_user_connections_owner_org FOREIGN KEY(org_id,virtual_user_id) REFERENCES virtual_users(org_id,id) ON DELETE CASCADE;
ALTER TABLE virtual_user_connections ADD CONSTRAINT virtual_user_connections_access
 FOREIGN KEY(org_id,id) REFERENCES resource_access(org_id,id) ON DELETE CASCADE DEFERRABLE INITIALLY DEFERRED;
ALTER TABLE credential_bindings ADD CONSTRAINT credential_source_tenant FOREIGN KEY(org_id,source_connection_id) REFERENCES resource_access(org_id,id);

-- Equivalent grants for existing explicit shared-App selections only.
INSERT INTO resource_grants(org_id,resource_id,consumer_scope,consumer_agent_id,usage,attachment_id)
 SELECT a.org_id,r.id,'agent',a.id,'service',c.id
 FROM agents a JOIN virtual_user_connections c ON c.virtual_user_id=a.virtual_user_id
 JOIN resource_access r ON r.id=(c.provider_metadata->>'org_connection_id')::uuid
 WHERE c.connection_type='everruns_app' AND c.provider_metadata ? 'org_connection_id'
 AND r.org_id=a.org_id AND r.scope='organization';

-- Legacy agent APIs remain adapters, while new scopes create their access row
-- transactionally. These triggers cover existing provider/agent writers too.
CREATE FUNCTION ensure_credential_resource_access() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE r resource_access%ROWTYPE;
BEGIN
 IF NEW.agent_id IS NOT NULL THEN
  NEW.owner_key='agent:'||NEW.agent_id::text||':service:';
 ELSE
  SELECT * INTO STRICT r FROM resource_access WHERE id=NEW.id AND org_id=NEW.org_id AND kind='credential_binding';
  NEW.owner_key=r.scope||':'||COALESCE(r.owner_agent_id,r.owner_session_id,r.owner_virtual_user_id)::text||':'||r.usage||':'||COALESCE(r.subject_id::text,'');
  IF r.scope='organization' THEN NEW.owner_key='organization:'||r.org_id::text||':'||r.usage||':'; END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER credential_resource_insert BEFORE INSERT ON credential_bindings
 FOR EACH ROW EXECUTE FUNCTION ensure_credential_resource_access();

-- AFTER INSERT runs only for an inserted row, avoiding orphan access headers
-- when a legacy writer's ON CONFLICT updates an existing credential/connection.
CREATE FUNCTION create_credential_resource_access() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF NEW.agent_id IS NOT NULL THEN
  INSERT INTO resource_access(id,org_id,kind,scope,owner_agent_id,usage)
   VALUES(NEW.id,NEW.org_id,'credential_binding','agent',NEW.agent_id,'service') ON CONFLICT(id) DO NOTHING;
  IF NOT EXISTS(SELECT 1 FROM resource_access WHERE id=NEW.id AND org_id=NEW.org_id AND kind='credential_binding' AND scope='agent' AND owner_agent_id=NEW.agent_id AND usage='service') THEN
   RAISE EXCEPTION 'Credential owner does not match its access record';
  END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER credential_resource_create AFTER INSERT ON credential_bindings
 FOR EACH ROW EXECUTE FUNCTION create_credential_resource_access();

CREATE FUNCTION ensure_connection_resource_access() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 SELECT org_id INTO STRICT NEW.org_id FROM virtual_users WHERE id=NEW.virtual_user_id;
 RETURN NEW;
END $$;
CREATE TRIGGER connection_resource_insert BEFORE INSERT ON virtual_user_connections
 FOR EACH ROW EXECUTE FUNCTION ensure_connection_resource_access();

CREATE FUNCTION create_connection_resource_access() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE v_usage TEXT;
BEGIN
 SELECT usage INTO STRICT v_usage FROM virtual_users WHERE id=NEW.virtual_user_id AND org_id=NEW.org_id;
 INSERT INTO resource_access(id,org_id,kind,scope,owner_virtual_user_id,usage)
 VALUES(NEW.id,NEW.org_id,'connection',CASE WHEN NEW.owner_scope='organization' THEN 'organization' ELSE 'virtual_user' END,
 CASE WHEN NEW.owner_scope='organization' THEN NULL ELSE NEW.virtual_user_id END,
 CASE WHEN v_usage='end_user' THEN 'user' ELSE 'service' END) ON CONFLICT(id) DO NOTHING;
 IF NOT EXISTS(SELECT 1 FROM resource_access WHERE id=NEW.id AND org_id=NEW.org_id AND kind='connection' AND ((scope='organization' AND NEW.owner_scope='organization') OR (scope='virtual_user' AND NEW.owner_scope='virtual_user' AND owner_virtual_user_id=NEW.virtual_user_id))) THEN
  RAISE EXCEPTION 'Connection owner does not match its access record';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER connection_resource_create AFTER INSERT ON virtual_user_connections
 FOR EACH ROW EXECUTE FUNCTION create_connection_resource_access();

CREATE FUNCTION bump_resource_revision() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 UPDATE resource_access SET revision=revision+1 WHERE id=NEW.id;
 RETURN NEW;
END $$;
CREATE TRIGGER credential_revision AFTER UPDATE OF value_encrypted,binding,expires_at,source_connection_id,source_github_access,env_name,mcp_server_name,mcp_server_url,tool_name,parameter_name ON credential_bindings
 FOR EACH ROW EXECUTE FUNCTION bump_resource_revision();
CREATE TRIGGER connection_revision AFTER UPDATE ON virtual_user_connections
 FOR EACH ROW EXECUTE FUNCTION bump_resource_revision();

CREATE FUNCTION remove_resource_access() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 DELETE FROM resource_access WHERE id=OLD.id;
 RETURN OLD;
END $$;
CREATE TRIGGER credential_resource_delete AFTER DELETE ON credential_bindings
 FOR EACH ROW EXECUTE FUNCTION remove_resource_access();
CREATE TRIGGER connection_resource_delete AFTER DELETE ON virtual_user_connections
 FOR EACH ROW EXECUTE FUNCTION remove_resource_access();

CREATE TABLE credential_capture_requests (
 id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
 org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
 session_id UUID NOT NULL,
 input_message_id UUID NOT NULL REFERENCES runtime_invocations(input_message_id) ON DELETE CASCADE,
 plan_hash TEXT NOT NULL,
 plan JSONB NOT NULL,
 targets JSONB NOT NULL,
 status TEXT NOT NULL CHECK(status IN ('pending','approved','executing','completed','declined')),
 authorizer_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
 expires_at TIMESTAMPTZ NOT NULL DEFAULT now()+interval '15 minutes',
 result JSONB,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
 FOREIGN KEY(org_id,session_id) REFERENCES sessions(org_id,id) ON DELETE CASCADE,
 UNIQUE(session_id,input_message_id,plan_hash)
);

-- Old bindings were explicitly session-wide service credentials. Preserve that
-- exact lifetime; never infer a personal virtual-user owner from the last speaker.
INSERT INTO resource_access(id,org_id,kind,scope,owner_session_id,usage)
 SELECT ss.id,s.org_id,'credential_binding','session',s.id,'service'
 FROM session_secrets ss JOIN sessions s ON s.id=ss.session_id WHERE ss.binding IS NOT NULL;
INSERT INTO credential_bindings(id,org_id,kind,env_name,binding,label,value_encrypted,created_at,updated_at)
 SELECT ss.id,s.org_id,'http',ss.name,ss.binding,ss.name,ss.value_encrypted,ss.created_at,ss.updated_at
 FROM session_secrets ss JOIN sessions s ON s.id=ss.session_id WHERE ss.binding IS NOT NULL;
DELETE FROM session_secrets WHERE binding IS NOT NULL;
-- A mixed-version worker cannot write into the retired bound-value store.
ALTER TABLE session_secrets ADD CONSTRAINT session_secrets_ordinary_only CHECK(binding IS NULL);

CREATE FUNCTION sync_shared_github_resource_grant() RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE a agents%ROWTYPE; source_id UUID;
BEGIN
 IF TG_OP='UPDATE' AND (NEW.connection_type IS DISTINCT FROM OLD.connection_type OR NEW.provider_metadata->>'org_connection_id' IS DISTINCT FROM OLD.provider_metadata->>'org_connection_id') THEN
  DELETE FROM resource_grants WHERE attachment_id=NEW.id AND org_id=NEW.org_id;
 END IF;
 IF NEW.connection_type='everruns_app' AND NEW.provider_metadata ? 'org_connection_id' THEN
  source_id=(NEW.provider_metadata->>'org_connection_id')::uuid;
  SELECT * INTO a FROM agents WHERE virtual_user_id=NEW.virtual_user_id;
  IF a.id IS NOT NULL THEN
   IF NOT EXISTS(SELECT 1 FROM resource_access WHERE id=source_id AND org_id=a.org_id AND kind='connection' AND scope='organization' AND revoked_at IS NULL) THEN
    RAISE EXCEPTION 'Shared installation is unavailable';
   END IF;
   IF TG_OP='INSERT' OR OLD.connection_type IS DISTINCT FROM NEW.connection_type OR OLD.provider_metadata->>'org_connection_id' IS DISTINCT FROM NEW.provider_metadata->>'org_connection_id' THEN
    DELETE FROM resource_grants WHERE attachment_id=NEW.id AND org_id=a.org_id;
    INSERT INTO resource_grants(org_id,resource_id,consumer_scope,consumer_agent_id,usage,attachment_id)
     VALUES(a.org_id,source_id,'agent',a.id,'service',NEW.id);
   END IF;
  END IF;
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER connection_shared_github_grant AFTER INSERT OR UPDATE OF provider_metadata,connection_type ON virtual_user_connections FOR EACH ROW EXECUTE FUNCTION sync_shared_github_resource_grant();

-- Payload-free history survives deletion without retaining credential values.
CREATE TABLE resource_access_audit (
 id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
 org_id BIGINT NOT NULL REFERENCES organizations(org_id) ON DELETE CASCADE,
 resource_id UUID NOT NULL,
 action TEXT NOT NULL,
 actor_user_id UUID REFERENCES users(id) ON DELETE SET NULL,
 input_message_id UUID,
 created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX resource_access_audit_resource ON resource_access_audit(org_id,resource_id,created_at DESC);

-- Transfer is not a grant operation. Payload writers cannot silently change
-- a resource's typed owner while retaining its old authorizations.
CREATE FUNCTION protect_resource_owner() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF ROW(NEW.org_id,NEW.kind,NEW.scope,NEW.owner_agent_id,NEW.owner_session_id,NEW.owner_virtual_user_id,NEW.subject_id,NEW.usage)
    IS DISTINCT FROM ROW(OLD.org_id,OLD.kind,OLD.scope,OLD.owner_agent_id,OLD.owner_session_id,OLD.owner_virtual_user_id,OLD.subject_id,OLD.usage) THEN
  RAISE EXCEPTION 'Resource ownership is immutable; create an explicit grant';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER resource_owner_immutable BEFORE UPDATE ON resource_access FOR EACH ROW EXECUTE FUNCTION protect_resource_owner();
CREATE FUNCTION protect_connection_owner() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF ROW(NEW.org_id,NEW.virtual_user_id,NEW.owner_scope) IS DISTINCT FROM ROW(OLD.org_id,OLD.virtual_user_id,OLD.owner_scope) THEN
  RAISE EXCEPTION 'Connection ownership is immutable';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER connection_owner_immutable BEFORE UPDATE ON virtual_user_connections FOR EACH ROW EXECUTE FUNCTION protect_connection_owner();
CREATE FUNCTION protect_credential_owner() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
 IF ROW(NEW.org_id,NEW.agent_id,NEW.owner_key,NEW.kind) IS DISTINCT FROM ROW(OLD.org_id,OLD.agent_id,OLD.owner_key,OLD.kind) THEN
  RAISE EXCEPTION 'Credential ownership is immutable';
 END IF;
 RETURN NEW;
END $$;
CREATE TRIGGER credential_owner_immutable BEFORE UPDATE ON credential_bindings FOR EACH ROW EXECUTE FUNCTION protect_credential_owner();
