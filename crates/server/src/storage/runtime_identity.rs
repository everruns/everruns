//! Verified runtime subjects; management identity is never a credential subject.
use super::{StorageBackend, models::*};
use anyhow::{Result, bail};
use everruns_provider::typed_id::{SessionId, VirtualUserId};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct VerifiedRuntimeIdentity {
    pub org_id: i64,
    pub provider: String,
    pub realm: String,
    pub subject: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub management_user_id: Option<Uuid>,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize)]
pub struct VirtualUserBindingRow {
    pub id: Uuid,
    pub org_id: i64,
    pub virtual_user_id: VirtualUserId,
    pub status: String,
    pub provider: String,
    pub realm: String,
    pub subject: String,
    pub management_user_id: Option<Uuid>,
}

impl StorageBackend {
    pub async fn register_connection_setup(
        &self,
        state: &str,
        provider: &str,
        hash: &[u8],
    ) -> Result<()> {
        if state.is_empty() || state.len() > 128 || provider.len() > 128 || hash.len() != 32 {
            bail!("Invalid setup state");
        }
        let expires = chrono::Utc::now() + chrono::Duration::minutes(10);
        match self {
            Self::Postgres(db) => {
                sqlx::query("DELETE FROM connection_setup_states WHERE state IN (SELECT state FROM connection_setup_states WHERE expires_at<now() LIMIT 1000)").execute(db.pool()).await?;
                sqlx::query("INSERT INTO connection_setup_states(state,provider,payload_hash,expires_at) VALUES($1,$2,$3,$4)").bind(state).bind(provider).bind(hash).bind(expires).execute(db.pool()).await?;
            }
            Self::InMemory(db) => {
                let mut setups = db.connection_setup_states.write();
                setups.retain(|_, s| s.2 > chrono::Utc::now());
                if setups.contains_key(state) {
                    bail!("Setup already exists");
                }
                setups.insert(state.into(), (provider.into(), hash.to_vec(), expires));
            }
        }
        Ok(())
    }
    pub async fn consume_connection_setup(
        &self,
        state: &str,
        provider: &str,
        hash: &[u8],
    ) -> Result<bool> {
        match self {
            Self::Postgres(db) => Ok(sqlx::query("DELETE FROM connection_setup_states WHERE state=$1 AND provider=$2 AND payload_hash=$3 AND expires_at>now()").bind(state).bind(provider).bind(hash).execute(db.pool()).await?.rows_affected()==1),
            Self::InMemory(db) => {
                let mut setups=db.connection_setup_states.write();
                if !setups.get(state).is_some_and(|s|s.0==provider && s.1==hash && s.2>chrono::Utc::now()) { return Ok(false); }
                setups.remove(state); Ok(true)
            }
        }
    }

    pub async fn resolve_runtime_identity(
        &self,
        input: VerifiedRuntimeIdentity,
    ) -> Result<VirtualUserRow> {
        if input.provider.is_empty()
            || input.realm.is_empty()
            || input.subject.is_empty()
            || input.provider.len() > 128
            || input.name.trim().is_empty()
            || input.subject.len() > 1024
            || input.realm.len() > 1024
            || input.name.len() > 255
        {
            bail!("Invalid runtime identity");
        }
        let key =
            serde_json::to_string(&(input.org_id, &input.provider, &input.realm, &input.subject))?;
        let id = VirtualUserId::from_uuid(Uuid::new_v5(&Uuid::NAMESPACE_URL, key.as_bytes()));
        let binding = match self {
            Self::Postgres(db) => sqlx::query_as::<_, VirtualUserBindingRow>("SELECT * FROM virtual_user_bindings WHERE org_id=$1 AND provider=$2 AND realm=$3 AND subject=$4")
                .bind(input.org_id).bind(&input.provider).bind(&input.realm).bind(&input.subject).fetch_optional(db.pool()).await?,
            Self::InMemory(db) => db.virtual_user_bindings.read().get(&key).cloned(),
        };
        if let Some(binding) = binding {
            if binding.status != "active" {
                bail!("Identity binding revoked");
            }
            return self
                .get_virtual_user(input.org_id, binding.virtual_user_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Runtime identity unavailable"));
        }
        // The deterministic id plus unique binding key arbitrate concurrent first use.
        self.create_virtual_user(CreateVirtualUserRow {
            org_id: input.org_id,
            id,
            usage: "end_user".into(),
            name: input.name,
            description: None,
            avatar_url: input.avatar_url,
            locale: None,
            timezone: None,
        })
        .await?;
        let binding = VirtualUserBindingRow {
            id: Uuid::now_v7(),
            org_id: input.org_id,
            virtual_user_id: id,
            status: "active".into(),
            provider: input.provider,
            realm: input.realm,
            subject: input.subject,
            management_user_id: input.management_user_id,
        };
        match self {
            Self::Postgres(db) => {
                sqlx::query("INSERT INTO virtual_user_bindings (id,org_id,virtual_user_id,provider,realm,subject,management_user_id) VALUES ($1,$2,$3,$4,$5,$6,$7) ON CONFLICT (org_id,provider,realm,subject) DO NOTHING")
                .bind(binding.id).bind(binding.org_id).bind(binding.virtual_user_id).bind(&binding.provider).bind(&binding.realm).bind(&binding.subject).bind(binding.management_user_id).execute(db.pool()).await?;
            }
            Self::InMemory(db) => {
                db.virtual_user_bindings
                    .write()
                    .entry(key.clone())
                    .or_insert(binding.clone());
            }
        }
        let winner=match self {
            Self::Postgres(db)=>sqlx::query_scalar::<_,VirtualUserId>("SELECT virtual_user_id FROM virtual_user_bindings WHERE org_id=$1 AND provider=$2 AND realm=$3 AND subject=$4").bind(binding.org_id).bind(&binding.provider).bind(&binding.realm).bind(&binding.subject).fetch_one(db.pool()).await?,
            Self::InMemory(db)=>db.virtual_user_bindings.read().get(&key).ok_or_else(||anyhow::anyhow!("Binding unavailable"))?.virtual_user_id,
        };
        self.get_virtual_user(input.org_id, winner)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Runtime identity unavailable"))
    }

    pub async fn list_pending_connections(&self, user: Uuid) -> Result<Vec<UserConnectionRow>> {
        match self {
            Self::Postgres(db)=>Ok(sqlx::query_as("SELECT * FROM pending_user_connections WHERE user_id=$1 ORDER BY provider LIMIT 100").bind(user).fetch_all(db.pool()).await?),
            Self::InMemory(db)=>Ok(db.pending_user_connections.read().values().filter(|c|c.user_id==user).cloned().collect()),
        }
    }
    pub async fn migrate_pending_connection(
        &self,
        org: i64,
        user: Uuid,
        connection: Uuid,
    ) -> Result<bool> {
        let target = self.default_virtual_user(org, user).await?;
        if target.status != "active" {
            bail!("Runtime account inactive");
        }
        match self {
            Self::Postgres(db) => {
                let mut tx = db.pool().begin().await?;
                let row = sqlx::query_as::<_, UserConnectionRow>(
                    "SELECT * FROM pending_user_connections WHERE id=$1 AND user_id=$2 FOR UPDATE",
                )
                .bind(connection)
                .bind(user)
                .fetch_optional(&mut *tx)
                .await?;
                let Some(c) = row else { return Ok(false) };
                let result=sqlx::query("INSERT INTO virtual_user_connections (id,virtual_user_id,provider,connection_type,provider_user_id,provider_username,access_token_encrypted,refresh_token_encrypted,scopes,expires_at,installation_id,provider_metadata,created_at,updated_at) SELECT $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14 FROM virtual_users v JOIN organization_members m ON m.org_id=v.org_id WHERE v.id=$2 AND v.org_id=$15 AND v.status='active' AND m.user_id=$16 ON CONFLICT(virtual_user_id,provider) DO NOTHING")
                    .bind(c.id).bind(target.id).bind(&c.provider).bind(&c.connection_type).bind(&c.provider_user_id).bind(&c.provider_username)
                    .bind(&c.access_token_encrypted).bind(&c.refresh_token_encrypted).bind(&c.scopes).bind(c.expires_at).bind(c.installation_id).bind(&c.provider_metadata).bind(c.created_at).bind(c.updated_at).bind(org).bind(user).execute(&mut *tx).await?;
                if result.rows_affected() != 1 {
                    bail!("Destination already has this provider or membership was revoked");
                }
                sqlx::query("UPDATE leased_resources SET pending_connection_id=NULL,metadata=metadata-'connection_migration_pending' WHERE pending_connection_id=$1 AND owner_user_id=$2").bind(c.id).bind(target.id).execute(&mut *tx).await?;
                // Other organizations keep their own owner and require an explicit new grant.
                sqlx::query("UPDATE leased_resources SET pending_connection_id=NULL WHERE pending_connection_id=$1").bind(c.id).execute(&mut *tx).await?;
                sqlx::query("DELETE FROM pending_user_connections WHERE id=$1 AND user_id=$2")
                    .bind(c.id)
                    .bind(user)
                    .execute(&mut *tx)
                    .await?;
                tx.commit().await?;
                Ok(true)
            }
            Self::InMemory(db) => {
                let mut pending = db.pending_user_connections.write();
                let Some(c) = pending
                    .get(&connection)
                    .filter(|c| c.user_id == user)
                    .cloned()
                else {
                    return Ok(false);
                };
                let mut active = db.virtual_user_connections.write();
                if active
                    .values()
                    .any(|r| r.virtual_user_id == target.id && r.provider == c.provider)
                {
                    bail!("Destination already has this provider");
                }
                active.insert(
                    c.id,
                    VirtualUserConnectionRow {
                        id: c.id,
                        virtual_user_id: target.id,
                        provider: c.provider.clone(),
                        connection_type: c.connection_type,
                        provider_user_id: c.provider_user_id,
                        provider_username: c.provider_username,
                        access_token_encrypted: c.access_token_encrypted,
                        refresh_token_encrypted: c.refresh_token_encrypted,
                        scopes: c.scopes,
                        expires_at: c.expires_at,
                        installation_id: c.installation_id,
                        provider_metadata: c.provider_metadata,
                        created_at: c.created_at,
                        updated_at: c.updated_at,
                    },
                );
                pending.remove(&connection);
                drop(active);
                drop(pending);
                for resource in db.leased_resources.write().values_mut().filter(|r| {
                    r.owner_user_id == Some(target.id.uuid()) && r.provider == c.provider
                }) {
                    if let Some(metadata) = resource.metadata.as_object_mut() {
                        metadata.remove("connection_migration_pending");
                    }
                }
                Ok(true)
            }
        }
    }

    pub async fn default_virtual_user(&self, org_id: i64, user_id: Uuid) -> Result<VirtualUserRow> {
        let user = self
            .get_user(user_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Management account not found"))?;
        if !self
            .list_user_organizations(user_id)
            .await?
            .iter()
            .any(|m| m.org_id == org_id)
        {
            bail!("Management account is not a member of this organization");
        }
        self.resolve_runtime_identity(VerifiedRuntimeIdentity {
            org_id,
            provider: "everruns".into(),
            realm: org_id.to_string(),
            subject: user_id.to_string(),
            name: user.name,
            avatar_url: user.avatar_url,
            management_user_id: Some(user_id),
        })
        .await
    }

    pub async fn list_virtual_user_bindings(
        &self,
        org_id: i64,
        id: VirtualUserId,
    ) -> Result<Vec<VirtualUserBindingRow>> {
        match self {
            Self::Postgres(db) => Ok(sqlx::query_as("SELECT * FROM virtual_user_bindings WHERE org_id=$1 AND virtual_user_id=$2 AND status='active' ORDER BY provider,realm,subject").bind(org_id).bind(id).fetch_all(db.pool()).await?),
            Self::InMemory(db) => Ok(db.virtual_user_bindings.read().values().filter(|b| b.org_id==org_id && b.virtual_user_id==id && b.status=="active").cloned().collect()),
        }
    }

    /// Keep a tombstone: the same external subject must never reclaim private state after unlinking.
    pub async fn revoke_virtual_user_binding(
        &self,
        org: i64,
        user: VirtualUserId,
        binding: Uuid,
    ) -> Result<bool> {
        match self {
            Self::Postgres(db) => Ok(sqlx::query("UPDATE virtual_user_bindings SET status='revoked' WHERE id=$1 AND org_id=$2 AND virtual_user_id=$3 AND management_user_id IS NULL AND status='active'").bind(binding).bind(org).bind(user).execute(db.pool()).await?.rows_affected()==1),
            Self::InMemory(db) => {
                let mut bindings=db.virtual_user_bindings.write();
                let Some(b)=bindings.values_mut().find(|b| b.id==binding && b.org_id==org && b.virtual_user_id==user && b.management_user_id.is_none() && b.status=="active") else {return Ok(false)};
                b.status="revoked".into(); Ok(true)
            }
        }
    }

    pub async fn list_virtual_user_sessions(
        &self,
        org: i64,
        user: VirtualUserId,
    ) -> Result<Vec<SessionRow>> {
        match self {
            Self::Postgres(db)=>Ok(sqlx::query_as("SELECT s.* FROM sessions s LEFT JOIN principals p ON p.id=s.owner_principal_id LEFT JOIN agents a ON a.id=s.agent_id WHERE s.org_id=$1 AND s.status<>'deleted' AND ((p.org_id=$1 AND p.kind='virtual_user' AND p.subject_id=$2) OR s.virtual_user_id=$2 OR a.virtual_user_id=$2 OR EXISTS(SELECT 1 FROM session_participants sp JOIN principals pp ON pp.id=sp.principal_id WHERE sp.org_id=$1 AND sp.session_id=s.id AND pp.kind='virtual_user' AND pp.subject_id=$2)) ORDER BY s.updated_at DESC LIMIT 100").bind(org).bind(user).fetch_all(db.pool()).await?),
            Self::InMemory(db)=>{
                let principals=db.principals.read();let agents=db.agents.read();let participants=db.session_participants.read();
                let mut sessions:Vec<_>=db.sessions.read().values().filter(|s|s.org_id==org && s.status!="deleted" && (s.virtual_user_id==Some(user) || principals.get(&s.owner_principal_id).is_some_and(|p|p.kind=="virtual_user" && p.subject_id==Some(user.uuid())) || s.agent_id.and_then(|id|agents.get(&id)).is_some_and(|a|a.virtual_user_id==Some(user)) || participants.values().any(|sp|sp.org_id==org && sp.session_id==s.id && principals.get(&sp.principal_id).is_some_and(|p|p.kind=="virtual_user" && p.subject_id==Some(user.uuid()))))).cloned().collect();
                sessions.sort_by_key(|s|std::cmp::Reverse(s.updated_at));sessions.truncate(100);Ok(sessions)
            }
        }
    }

    pub async fn rotate_runtime_connection(
        &self,
        expected: &[u8],
        input: UpdateOAuthConnectionTokens,
    ) -> Result<Option<VirtualUserConnectionRow>> {
        match self {
            Self::Postgres(db)=>Ok(sqlx::query_as("UPDATE virtual_user_connections SET access_token_encrypted=$3,refresh_token_encrypted=$4,expires_at=$5,scopes=COALESCE($6,scopes),updated_at=now() WHERE id=$1 AND access_token_encrypted=$2 AND connection_type='oauth' RETURNING *").bind(input.connection_id).bind(expected).bind(input.access_token_encrypted).bind(input.refresh_token_encrypted).bind(input.expires_at).bind(input.scopes).fetch_optional(db.pool()).await?),
            Self::InMemory(db)=>{
                let mut rows=db.virtual_user_connections.write();let Some(row)=rows.get_mut(&input.connection_id).filter(|r|r.connection_type=="oauth" && r.access_token_encrypted.as_deref()==Some(expected)) else {return Ok(None)};
                row.access_token_encrypted=Some(input.access_token_encrypted);row.refresh_token_encrypted=Some(input.refresh_token_encrypted);row.expires_at=input.expires_at;if input.scopes.is_some(){row.scopes=input.scopes}row.updated_at=chrono::Utc::now();Ok(Some(row.clone()))
            }
        }
    }
    pub async fn revoke_runtime_connection_if_unchanged(
        &self,
        id: Uuid,
        expected: &[u8],
    ) -> Result<bool> {
        match self {
            Self::Postgres(db) => Ok(sqlx::query(
                "DELETE FROM virtual_user_connections WHERE id=$1 AND access_token_encrypted=$2",
            )
            .bind(id)
            .bind(expected)
            .execute(db.pool())
            .await?
            .rows_affected()
                == 1),
            Self::InMemory(db) => {
                let mut rows = db.virtual_user_connections.write();
                if rows
                    .get(&id)
                    .is_some_and(|r| r.access_token_encrypted.as_deref() == Some(expected))
                {
                    rows.remove(&id);
                    Ok(true)
                } else {
                    Ok(false)
                }
            }
        }
    }
    pub async fn rotate_runtime_session_grant(
        &self,
        expected: &McpOAuthSessionCredentialsRow,
        input: UpsertMcpOAuthSessionCredentials,
    ) -> Result<bool> {
        let access = everruns_core::mcp_oauth_session_secret_name(input.server_id, "access_token");
        let fields = [
            (access.clone(), Some(input.access_token_encrypted)),
            (
                everruns_core::mcp_oauth_session_secret_name(input.server_id, "refresh_token"),
                input.refresh_token_encrypted,
            ),
            (
                everruns_core::mcp_oauth_session_secret_name(input.server_id, "expires_at"),
                input.expires_at_encrypted,
            ),
        ];
        match self {
            Self::Postgres(db) => {
                let mut tx = db.pool().begin().await?;
                let current: Option<SessionSecretRow> = sqlx::query_as(
                    "SELECT * FROM session_secrets WHERE session_id=$1 AND name=$2 FOR UPDATE",
                )
                .bind(input.session_id)
                .bind(access)
                .fetch_optional(&mut *tx)
                .await?;
                if !current.is_some_and(|r| {
                    r.virtual_user_id == expected.virtual_user_id
                        && r.value_encrypted == expected.access_token_encrypted
                }) {
                    return Ok(false);
                }
                for (name, value) in fields {
                    if let Some(value) = value {
                        sqlx::query("INSERT INTO session_secrets(session_id,name,value_encrypted,virtual_user_id) VALUES($1,$2,$3,$4) ON CONFLICT(session_id,name) DO UPDATE SET value_encrypted=EXCLUDED.value_encrypted,virtual_user_id=EXCLUDED.virtual_user_id,updated_at=now()").bind(input.session_id).bind(name).bind(value).bind(input.virtual_user_id).execute(&mut *tx).await?;
                    } else {
                        sqlx::query("DELETE FROM session_secrets WHERE session_id=$1 AND name=$2")
                            .bind(input.session_id)
                            .bind(name)
                            .execute(&mut *tx)
                            .await?;
                    }
                }
                tx.commit().await?;
                Ok(true)
            }
            Self::InMemory(db) => {
                let mut rows = db.session_secrets.write();
                if !rows.get(&(input.session_id, access)).is_some_and(|r| {
                    r.virtual_user_id == expected.virtual_user_id
                        && r.value_encrypted == expected.access_token_encrypted
                }) {
                    return Ok(false);
                }
                for (name, value) in fields {
                    let key = (input.session_id, name.clone());
                    if let Some(value) = value {
                        let now = chrono::Utc::now();
                        let row = rows.entry(key).or_insert(SessionSecretRow {
                            id: Uuid::now_v7(),
                            session_id: input.session_id,
                            name,
                            value_encrypted: vec![],
                            virtual_user_id: input.virtual_user_id,
                            created_at: now,
                            updated_at: now,
                        });
                        row.value_encrypted = value;
                        row.virtual_user_id = input.virtual_user_id;
                        row.updated_at = now;
                    } else {
                        rows.remove(&key);
                    }
                }
                Ok(true)
            }
        }
    }

    pub async fn record_runtime_invocation(
        &self,
        org_id: i64,
        session_id: SessionId,
        message_id: Uuid,
        subject: Option<VirtualUserId>,
        management_user_id: Option<Uuid>,
        responder_agent_id: Option<Uuid>,
    ) -> Result<()> {
        if let Some(id) = subject {
            let row = self
                .get_virtual_user(org_id, id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Runtime subject not found"))?;
            if row.status != "active" || row.usage != "end_user" {
                bail!("Runtime subject is not an active end user");
            }
        }
        if let Some(agent) = responder_agent_id
            && self
                .get_agent(org_id, agent.into())
                .await?
                .is_none_or(|a| a.status != "active")
        {
            bail!("Responder not available in this organization");
        }
        if let Some(user) = management_user_id {
            let v = self.default_virtual_user(org_id, user).await?;
            if subject != Some(v.id) {
                bail!("Management authorization must match the verified default runtime subject");
            }
        }
        match self {
            Self::Postgres(db) => {
                let result=sqlx::query("INSERT INTO runtime_invocations (input_message_id,org_id,session_id,virtual_user_id,management_user_id,responder_agent_id) SELECT $1,$2,$3,$4,$5,$6 FROM sessions WHERE id=$3 AND org_id=$2 ON CONFLICT (input_message_id) DO UPDATE SET input_message_id=EXCLUDED.input_message_id WHERE runtime_invocations.org_id=EXCLUDED.org_id AND runtime_invocations.session_id=EXCLUDED.session_id AND runtime_invocations.virtual_user_id IS NOT DISTINCT FROM EXCLUDED.virtual_user_id AND runtime_invocations.management_user_id IS NOT DISTINCT FROM EXCLUDED.management_user_id AND runtime_invocations.responder_agent_id IS NOT DISTINCT FROM EXCLUDED.responder_agent_id")
                .bind(message_id).bind(org_id).bind(session_id).bind(subject).bind(management_user_id).bind(responder_agent_id).execute(db.pool()).await?;
                if result.rows_affected() != 1 {
                    bail!("Invocation not found or identity cannot be changed");
                }
            }
            Self::InMemory(db) => {
                if db
                    .sessions
                    .read()
                    .get(&session_id)
                    .is_none_or(|s| s.org_id != org_id)
                {
                    bail!("Session not found");
                }
                let value = (
                    org_id,
                    session_id,
                    subject,
                    management_user_id,
                    responder_agent_id,
                );
                let mut invocations = db.runtime_invocations.write();
                if let Some(existing) = invocations.get(&message_id)
                    && *existing != value
                {
                    bail!("Invocation identity cannot be changed");
                }
                invocations.entry(message_id).or_insert(value);
            }
        }
        Ok(())
    }

    pub async fn runtime_invocation_exists(
        &self,
        session: SessionId,
        message: Uuid,
    ) -> Result<bool> {
        match self {
            Self::Postgres(db)=>Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runtime_invocations WHERE session_id=$1 AND input_message_id=$2)").bind(session).bind(message).fetch_one(db.pool()).await?),
            Self::InMemory(db)=>Ok(db.runtime_invocations.read().get(&message).is_some_and(|r|r.1==session)),
        }
    }
    pub async fn runtime_invocation_has_subject(
        &self,
        session: SessionId,
        message: Uuid,
    ) -> Result<bool> {
        match self {
            Self::Postgres(db)=>Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM runtime_invocations WHERE session_id=$1 AND input_message_id=$2 AND virtual_user_id IS NOT NULL)").bind(session).bind(message).fetch_one(db.pool()).await?),
            Self::InMemory(db)=>Ok(db.runtime_invocations.read().get(&message).is_some_and(|r|r.1==session && r.2.is_some())),
        }
    }
    pub async fn runtime_invocation_responder(
        &self,
        session: SessionId,
        message: Uuid,
    ) -> Result<Option<Uuid>> {
        match self {
         Self::Postgres(db)=>Ok(sqlx::query_scalar::<_,Option<Uuid>>("SELECT responder_agent_id FROM runtime_invocations WHERE session_id=$1 AND input_message_id=$2").bind(session).bind(message).fetch_optional(db.pool()).await?.flatten()),
         Self::InMemory(db)=>Ok(db.runtime_invocations.read().get(&message).filter(|r|r.1==session).and_then(|r|r.4))
        }
    }
    pub async fn runtime_invocation_management_user(
        &self,
        session: SessionId,
        message: Uuid,
    ) -> Result<Option<Uuid>> {
        match self {
         Self::Postgres(db)=>Ok(sqlx::query_scalar::<_,Option<Uuid>>("SELECT management_user_id FROM runtime_invocations WHERE session_id=$1 AND input_message_id=$2").bind(session).bind(message).fetch_optional(db.pool()).await?.flatten()),
         Self::InMemory(db)=>Ok(db.runtime_invocations.read().get(&message).filter(|r|r.1==session).and_then(|r|r.3))
        }
    }
    pub async fn runtime_invocation_subject(
        &self,
        session_id: SessionId,
        message_id: Uuid,
    ) -> Result<Option<VirtualUserId>> {
        match self {
            Self::Postgres(db) => Ok(sqlx::query_scalar::<_, Option<VirtualUserId>>("SELECT r.virtual_user_id FROM runtime_invocations r JOIN virtual_users v ON v.id=r.virtual_user_id AND v.org_id=r.org_id WHERE r.session_id=$1 AND r.input_message_id=$2 AND v.status='active' AND v.usage='end_user'").bind(session_id).bind(message_id).fetch_optional(db.pool()).await?.flatten()),
            Self::InMemory(db) => { let id=db.runtime_invocations.read().get(&message_id).filter(|r| r.1==session_id).and_then(|r|r.2);
                Ok(id.filter(|id| db.virtual_users.read().get(id).is_some_and(|v|v.status=="active" && v.usage=="end_user"))) },
        }
    }
}

#[cfg(test)]
#[path = "runtime_identity_tests.rs"]
mod tests;
