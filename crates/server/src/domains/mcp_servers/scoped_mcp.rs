// Scoped MCP helpers.
//
// Decision: harness/agent/session-scoped remote MCP servers are merged with
// last-wins semantics by logical name, then resolved ahead of org-scoped MCP
// servers. Tool discovery is live (no persisted cache) to keep this feature
// narrowly scoped and avoid mutating config rows during runtime.

use crate::kernel_imports::{
    Capability, EgressService, McpProtocolMode, McpServerActsAs, McpServerAuthMode,
    McpServerTransportType, ScopedMcpServer, ScopedMcpServers,
    everruns_provider::tool_types::ToolDefinition, everruns_provider::typed_id::SessionId,
    everruns_provider::url_validation::validate_safe_url, merge_scoped_mcp_servers,
    resolve_runtime_capabilities,
};
use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use everruns_core::capabilities::{CapabilityRegistry, collect_capability_mcp_servers};
use everruns_core::connection_services::UserConnectionResolver;
use everruns_core::mcp_server::sanitize_mcp_server_name;
use everruns_mcp::{CacheHints, CacheScope, McpCapability};
use everruns_platform::{Agent, Harness, Session};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;
use tokio::sync::Mutex as AsyncMutex;
use uuid::Uuid;

use crate::domains::mcp_servers::McpServerResolved;
use crate::domains::mcp_servers::service::{
    McpServerService, fetch_mcp_tools, fetch_mcp_tools_with_cache_hints,
};
use crate::storage::{McpServiceToolCacheRow, StorageBackend, UpsertMcpServiceToolCache};

const SCOPED_TOOL_CACHE_MAX_AGE: Duration = Duration::from_secs(24 * 60 * 60);

#[derive(Debug, Clone, Copy)]
pub struct ScopedMcpCacheContext {
    pub agent_id: Option<Uuid>,
    pub user_id: Option<Uuid>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum CacheIdentity {
    Service {
        org_id: i64,
        preset_id: Uuid,
        agent_id: Uuid,
    },
    User {
        org_id: i64,
        preset_id: Uuid,
        user_id: Uuid,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ScopedToolCacheKey {
    identity: CacheIdentity,
    scope: CacheScopeKey,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum CacheScopeKey {
    Public,
    Private(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ScopedRefreshKey {
    identity: CacheIdentity,
    credential_hash: String,
}

#[derive(Debug, Clone)]
struct CachedScopedTools {
    tools: Vec<everruns_core::McpToolDefinition>,
    ttl: Duration,
    cached_at: DateTime<Utc>,
}

impl CachedScopedTools {
    fn age(&self) -> Option<chrono::Duration> {
        const FUTURE_SKEW_SECS: i64 = 300;
        let age = Utc::now().signed_duration_since(self.cached_at);
        if age < chrono::Duration::seconds(-FUTURE_SKEW_SECS) {
            return None;
        }
        Some(age.max(chrono::Duration::zero()))
    }

    fn is_fresh(&self) -> bool {
        self.age().is_some_and(|age| {
            age < chrono::Duration::from_std(self.ttl)
                .unwrap_or_else(|_| chrono::Duration::hours(24))
                && age
                    < chrono::Duration::from_std(SCOPED_TOOL_CACHE_MAX_AGE)
                        .unwrap_or_else(|_| chrono::Duration::hours(24))
        })
    }

    fn is_within_max_age(&self) -> bool {
        self.age().is_some_and(|age| {
            age < chrono::Duration::from_std(SCOPED_TOOL_CACHE_MAX_AGE)
                .unwrap_or_else(|_| chrono::Duration::hours(24))
        })
    }
}

static USER_TOOL_CACHE: LazyLock<Mutex<HashMap<ScopedToolCacheKey, CachedScopedTools>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static SCOPED_REFRESH_LOCKS: LazyLock<ScopedRefreshLocks> =
    LazyLock::new(ScopedRefreshLocks::default);

#[derive(Default)]
struct ScopedRefreshLocks {
    locks: Mutex<HashMap<ScopedRefreshKey, Arc<AsyncMutex<()>>>>,
}

impl ScopedRefreshLocks {
    fn lock_for(&self, key: ScopedRefreshKey) -> Arc<AsyncMutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(|error| error.into_inner());
        locks.retain(|existing, lock| *existing == key || Arc::strong_count(lock) > 1);
        locks.entry(key).or_default().clone()
    }
}

pub fn merge_effective_scoped_mcp_servers(
    harness: &Harness,
    agent: Option<&Agent>,
    session: &Session,
) -> ScopedMcpServers {
    let mut layers = vec![&harness.mcp_servers];
    if let Some(agent) = agent {
        layers.push(&agent.mcp_servers);
    }
    layers.push(&session.mcp_servers);

    let merged = merge_scoped_mcp_server_layers(layers);
    strip_untrusted_oauth_from_scoped_mcp_servers(&merged)
}

/// Sanitize explicit (user-controlled) scoped MCP servers so they cannot
/// request OAuth connection tokens for runtime tool discovery.
///
/// Capability-contributed servers are trusted (built-in code) and bypass this
/// step entirely — see `merge_effective_scoped_mcp_servers_with_capabilities`.
///
/// Behavior is intentionally narrow: only entries that were explicitly
/// configured as `auth_mode = OAuth` are altered. We clear `oauth_provider_id`
/// (so no token resolution happens) and switch `auth_mode` back to `None`. We
/// also disable `tool_discovery` on those entries — without a token, an
/// authenticated tools/list would 401, so attempting it would just spam logs
/// and slow scoped-MCP wiring.
fn strip_untrusted_oauth_from_scoped_mcp_servers(servers: &ScopedMcpServers) -> ScopedMcpServers {
    servers
        .iter()
        .map(|(name, server)| {
            let mut server = server.clone();
            if matches!(server.auth_mode, McpServerAuthMode::OAuth) {
                server.auth_mode = McpServerAuthMode::None;
                server.oauth_provider_id = None;
                server.tool_discovery = false;
            }
            (name.clone(), server)
        })
        .collect()
}
pub fn merge_effective_scoped_mcp_servers_with_capabilities(
    harness: &Harness,
    agent: Option<&Agent>,
    session: &Session,
    capability_registry: &CapabilityRegistry,
) -> ScopedMcpServers {
    let explicit = merge_effective_scoped_mcp_servers(harness, agent, session);
    // Status-agnostic projection: scoped-MCP wiring historically saw the
    // stored records regardless of lifecycle status (EVE-877, EVE-881).
    let agent_definition = agent.map(|a| a.definition());
    let harness_definition = harness.definition();
    // EVE-882: capability resolution consumes the portable execution view.
    let execution_session = session.execution_session();
    let resolved = resolve_runtime_capabilities(
        &harness_definition,
        agent_definition.as_ref(),
        &execution_session,
        capability_registry,
    );
    let contributed =
        collect_capability_mcp_servers(&resolved.resolved_capability_configs, capability_registry);

    merge_scoped_mcp_servers(&contributed, &explicit)
}

pub fn merge_scoped_mcp_server_layers<'a, I>(layers: I) -> ScopedMcpServers
where
    I: IntoIterator<Item = &'a ScopedMcpServers>,
{
    let mut merged = ScopedMcpServers::default();
    for layer in layers {
        merged = merge_scoped_mcp_servers(&merged, layer);
    }
    merged
}

pub fn validate_merged_scoped_mcp_servers<'a, I>(layers: I) -> Result<ScopedMcpServers>
where
    I: IntoIterator<Item = &'a ScopedMcpServers>,
{
    let merged = merge_scoped_mcp_server_layers(layers);
    validate_scoped_mcp_servers(&merged)?;
    Ok(merged)
}

pub async fn resolve_scoped_mcp_server(
    mcp_server_service: &McpServerService,
    org_id: i64,
    harness: &Harness,
    agent: Option<&Agent>,
    session: &Session,
    server_prefix: &str,
) -> Result<Option<McpServerResolved>> {
    let effective = merge_effective_scoped_mcp_servers(harness, agent, session);
    let matched = effective.into_iter().find(|(name, _)| {
        everruns_core::mcp_server::is_valid_mcp_server_name(name)
            && sanitize_mcp_server_name(name) == server_prefix
    });
    resolve_matched_scoped_mcp_server(mcp_server_service, org_id, session.id.uuid(), matched).await
}

pub async fn resolve_scoped_mcp_server_with_capabilities(
    mcp_server_service: &McpServerService,
    org_id: i64,
    harness: &Harness,
    agent: Option<&Agent>,
    session: &Session,
    server_prefix: &str,
    capability_registry: &CapabilityRegistry,
) -> Result<Option<McpServerResolved>> {
    let effective = merge_effective_scoped_mcp_servers_with_capabilities(
        harness,
        agent,
        session,
        capability_registry,
    );
    let matched = effective.into_iter().find(|(name, _)| {
        everruns_core::mcp_server::is_valid_mcp_server_name(name)
            && sanitize_mcp_server_name(name) == server_prefix
    });
    resolve_matched_scoped_mcp_server(mcp_server_service, org_id, session.id.uuid(), matched).await
}

async fn resolve_matched_scoped_mcp_server(
    mcp_server_service: &McpServerService,
    org_id: i64,
    session_id: Uuid,
    matched: Option<(String, ScopedMcpServer)>,
) -> Result<Option<McpServerResolved>> {
    let Some((name, server)) = matched else {
        return Ok(None);
    };
    if let Some(preset) = &server.preset {
        let preset_name = preset.catalog_name();
        let mut resolved = mcp_server_service
            .resolve_transport_by_name(&everruns_core::Caller::internal(org_id), preset_name)
            .await?
            .ok_or_else(|| {
                anyhow!("Catalog MCP server preset '{preset_name}' is missing or not active")
            })?;
        // The catalog row's own id is the connection-store key; the descriptor
        // id is rewritten below to the session-scoped one, so capture it first.
        let preset_server_id = resolved.id;
        resolved.id = scoped_mcp_server_uuid(session_id, &name);
        resolved.name = name;
        resolved.acts_as = server.acts_as;

        if !server.acts_as.is_none() {
            // A non-`none` attachment resolves its credential from a connection
            // store keyed by the preset, never from the preset's own config.
            resolved.auth_mode = McpServerAuthMode::OAuth;
            resolved.oauth_provider_id = Some(everruns_core::mcp_oauth_provider_id_for_uuid(
                preset_server_id,
            ));

            if server.acts_as == McpServerActsAs::User {
                // THREAT[TM-TOOL-041]: a `user` attachment can never carry
                // service auth. Dropping these here rather than at validation
                // means a config written before validation existed, or written
                // straight to the database, still cannot present an org-held
                // credential as the invoking user (EVE-1029, D2).
                resolved.api_key = None;
                resolved
                    .headers
                    .retain(|key, _| !key.eq_ignore_ascii_case("authorization"));
            }
        }

        return Ok(Some(resolved));
    }

    Ok(Some(McpServerResolved {
        id: scoped_mcp_server_uuid(session_id, &name),
        name,
        url: server.url,
        auth_mode: server.auth_mode,
        protocol_mode: server.protocol_mode,
        oauth_provider_id: server.oauth_provider_id,
        acts_as: server.acts_as,
        api_key: None,
        headers: server.headers,
    }))
}

pub async fn materialize_scoped_mcp_servers(
    db: &StorageBackend,
    org_id: i64,
    servers: &ScopedMcpServers,
) -> Result<ScopedMcpServers> {
    let mut materialized = ScopedMcpServers::new();
    for (name, server) in servers {
        let Some(preset) = &server.preset else {
            materialized.insert(name.clone(), server.clone());
            continue;
        };
        let preset_name = preset.catalog_name();
        let row = db
            .get_mcp_server_by_name(org_id, preset_name)
            .await?
            .filter(|row| row.status == "active")
            .ok_or_else(|| {
                anyhow!("Catalog MCP server preset '{preset_name}' is missing or not active")
            })?;
        let settings = McpServerService::settings_from_row(&row);
        materialized.insert(
            name.clone(),
            ScopedMcpServer {
                transport_type: McpServerTransportType::from(row.transport_type.as_str()),
                url: row.url,
                headers: serde_json::from_value(row.headers).unwrap_or_default(),
                protocol_mode: settings.protocol_mode,
                acts_as: server.acts_as,
                ..Default::default()
            },
        );
    }
    Ok(materialized)
}

enum CacheLookup {
    Fresh(Vec<everruns_core::McpToolDefinition>),
    Stale(Vec<everruns_core::McpToolDefinition>),
    Expired,
    Miss,
}

fn credential_hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

fn cache_identity(
    org_id: i64,
    preset_id: Uuid,
    acts_as: McpServerActsAs,
    context: ScopedMcpCacheContext,
) -> Option<CacheIdentity> {
    match acts_as {
        McpServerActsAs::Service => context.agent_id.map(|agent_id| CacheIdentity::Service {
            org_id,
            preset_id,
            agent_id,
        }),
        McpServerActsAs::User => context.user_id.map(|user_id| CacheIdentity::User {
            org_id,
            preset_id,
            user_id,
        }),
        McpServerActsAs::None => None,
    }
}

fn cached_tools_from_service_row(row: McpServiceToolCacheRow) -> CachedScopedTools {
    CachedScopedTools {
        tools: serde_json::from_value(row.cached_tools).unwrap_or_default(),
        ttl: Duration::from_millis(row.ttl_ms.max(0) as u64),
        cached_at: row.tools_cached_at,
    }
}

async fn load_cache_entry(
    db: &StorageBackend,
    key: &ScopedToolCacheKey,
) -> Result<Option<CachedScopedTools>> {
    match &key.identity {
        CacheIdentity::Service {
            org_id,
            preset_id,
            agent_id,
        } => {
            let (scope, hash) = match &key.scope {
                CacheScopeKey::Public => ("public", ""),
                CacheScopeKey::Private(hash) => ("private", hash.as_str()),
            };
            Ok(db
                .get_mcp_service_tool_cache(*org_id, *preset_id, *agent_id, scope, hash)
                .await?
                .map(cached_tools_from_service_row))
        }
        CacheIdentity::User { .. } => Ok(USER_TOOL_CACHE
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(key)
            .cloned()),
    }
}

async fn lookup_cached_tools(
    db: &StorageBackend,
    identity: CacheIdentity,
    hash: &str,
) -> Result<CacheLookup> {
    let mut stale = None;
    let mut expired = false;
    for scope in [
        CacheScopeKey::Public,
        CacheScopeKey::Private(hash.to_string()),
    ] {
        let key = ScopedToolCacheKey { identity, scope };
        let Some(entry) = load_cache_entry(db, &key).await? else {
            continue;
        };
        if entry.is_fresh() {
            return Ok(CacheLookup::Fresh(entry.tools));
        }
        if entry.is_within_max_age() {
            stale.get_or_insert(entry.tools);
        } else {
            expired = true;
        }
    }
    if let Some(tools) = stale {
        Ok(CacheLookup::Stale(tools))
    } else if expired {
        Ok(CacheLookup::Expired)
    } else {
        Ok(CacheLookup::Miss)
    }
}

async fn invalidate_identity_cache(db: &StorageBackend, identity: CacheIdentity) -> Result<()> {
    match identity {
        CacheIdentity::Service {
            org_id,
            preset_id,
            agent_id,
        } => {
            db.delete_mcp_service_tool_caches(org_id, preset_id, agent_id)
                .await?;
        }
        CacheIdentity::User { .. } => {
            USER_TOOL_CACHE
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .retain(|key, _| key.identity != identity);
        }
    }
    Ok(())
}

async fn store_cached_tools(
    db: &StorageBackend,
    identity: CacheIdentity,
    hash: &str,
    hints: CacheHints,
    tools: &[everruns_core::McpToolDefinition],
) -> Result<()> {
    let (scope, credential_hash) = match hints.scope {
        CacheScope::Public => (CacheScopeKey::Public, String::new()),
        CacheScope::Private => (CacheScopeKey::Private(hash.to_string()), hash.to_string()),
    };
    let entry = CachedScopedTools {
        tools: tools.to_vec(),
        ttl: hints.ttl,
        cached_at: Utc::now(),
    };
    match identity {
        CacheIdentity::Service {
            org_id,
            preset_id,
            agent_id,
        } => {
            db.upsert_mcp_service_tool_cache(UpsertMcpServiceToolCache {
                org_id,
                mcp_server_id: preset_id,
                agent_id,
                cache_scope: match scope {
                    CacheScopeKey::Public => "public",
                    CacheScopeKey::Private(_) => "private",
                }
                .to_string(),
                credential_hash,
                cached_tools: serde_json::to_value(tools)?,
                ttl_ms: hints.ttl.as_millis().min(i64::MAX as u128) as i64,
            })
            .await?;
        }
        CacheIdentity::User { .. } => {
            let mut cache = USER_TOOL_CACHE
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            cache.retain(|_, entry| entry.is_within_max_age());
            cache.insert(ScopedToolCacheKey { identity, scope }, entry);
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn discover_catalog_tools(
    db: &StorageBackend,
    org_id: i64,
    preset_id: Uuid,
    server_name: &str,
    server: &ScopedMcpServer,
    session_id: SessionId,
    context: ScopedMcpCacheContext,
    connection_resolver: &Arc<dyn UserConnectionResolver>,
    egress_service: &dyn EgressService,
) -> Result<Option<Vec<everruns_core::McpToolDefinition>>> {
    let Some(identity) = cache_identity(org_id, preset_id, server.acts_as, context) else {
        return Ok(None);
    };
    let provider = everruns_core::mcp_oauth_provider_id_for_uuid(preset_id);
    let token = connection_resolver
        .get_mcp_connection_token(session_id, &provider, server.acts_as)
        .await
        .map_err(|error| anyhow!("Failed to resolve scoped MCP discovery token: {error}"))?;
    let Some(token) = token else {
        invalidate_identity_cache(db, identity).await?;
        return Ok(None);
    };
    let hash = credential_hash(&token);
    match lookup_cached_tools(db, identity, &hash).await? {
        CacheLookup::Fresh(tools) => return Ok(Some(tools)),
        CacheLookup::Expired => {
            invalidate_identity_cache(db, identity).await?;
            return Ok(None);
        }
        CacheLookup::Stale(_) | CacheLookup::Miss => {}
    }

    let refresh_key = ScopedRefreshKey {
        identity,
        credential_hash: hash.clone(),
    };
    let lock = SCOPED_REFRESH_LOCKS.lock_for(refresh_key);
    let _guard = lock.lock_owned().await;
    match lookup_cached_tools(db, identity, &hash).await? {
        CacheLookup::Fresh(tools) => return Ok(Some(tools)),
        CacheLookup::Expired => {
            invalidate_identity_cache(db, identity).await?;
            return Ok(None);
        }
        CacheLookup::Stale(stale) => {
            match fetch_mcp_tools_with_cache_hints(
                egress_service,
                &server.url,
                Some(&token),
                &server.headers,
            )
            .await
            {
                Ok(result) => {
                    if let Some(hints) = result.cache_hints {
                        store_cached_tools(db, identity, &hash, hints, &result.tools).await?;
                    }
                    return Ok(Some(result.tools));
                }
                Err(error) => {
                    tracing::warn!(
                        server_name,
                        %error,
                        "Failed to refresh scoped MCP tool cache; serving cached tools within maximum age"
                    );
                    return Ok(Some(stale));
                }
            }
        }
        CacheLookup::Miss => {}
    }

    let result = fetch_mcp_tools_with_cache_hints(
        egress_service,
        &server.url,
        Some(&token),
        &server.headers,
    )
    .await?;
    if let Some(hints) = result.cache_hints {
        store_cached_tools(db, identity, &hash, hints, &result.tools).await?;
    }
    Ok(Some(result.tools))
}

pub async fn build_materialized_scoped_mcp_tool_definitions(
    db: &StorageBackend,
    org_id: i64,
    servers: &ScopedMcpServers,
    session_id: Option<SessionId>,
    connection_resolver: Option<&Arc<dyn UserConnectionResolver>>,
    egress_service: &dyn EgressService,
) -> Result<Vec<ToolDefinition>> {
    let materialized = materialize_scoped_mcp_servers(db, org_id, servers).await?;
    let cache_context = match session_id {
        Some(session_id) => {
            db.get_session(org_id, session_id)
                .await?
                .map(|session| ScopedMcpCacheContext {
                    agent_id: session.agent_id.map(|id| id.uuid()),
                    user_id: session.resolved_owner_user_id,
                })
        }
        None => None,
    };
    let mut definitions = Vec::new();
    for (name, server) in &materialized {
        let source = servers
            .get(name)
            .expect("materialized server keeps its name");
        let runtime_identity_attachment =
            !server.acts_as.is_none() && source.preset.is_some() && session_id.is_some();
        if !runtime_identity_attachment {
            definitions.extend(
                build_scoped_mcp_tool_definitions(
                    &ScopedMcpServers::from([(name.clone(), server.clone())]),
                    session_id,
                    connection_resolver,
                    egress_service,
                )
                .await?,
            );
            continue;
        }
        let (Some(cache_context), Some(connection_resolver)) = (cache_context, connection_resolver)
        else {
            tracing::warn!(
                server_name = %name,
                acts_as = %server.acts_as,
                "Skipping catalog MCP discovery because runtime identity context is unavailable"
            );
            continue;
        };

        let preset_name = source
            .preset
            .as_ref()
            .expect("cacheable catalog attachment has a preset")
            .catalog_name();
        let preset_id = db
            .get_mcp_server_by_name(org_id, preset_name)
            .await?
            .filter(|row| row.status == "active")
            .ok_or_else(|| {
                anyhow!("Catalog MCP server preset '{preset_name}' is missing or not active")
            })?
            .id
            .uuid();
        let tools = discover_catalog_tools(
            db,
            org_id,
            preset_id,
            name,
            server,
            session_id.expect("runtime identity attachment has a session"),
            cache_context,
            connection_resolver,
            egress_service,
        )
        .await;
        let tools = match tools {
            Ok(Some(tools)) => tools,
            Ok(None) => continue,
            Err(error) => {
                tracing::warn!(
                    server_name = %name,
                    %error,
                    "Failed to discover cached catalog MCP tools, skipping server"
                );
                continue;
            }
        };
        let capability_id = session_id
            .map(|id| scoped_mcp_server_uuid(id.uuid(), name))
            .unwrap_or_else(Uuid::nil);
        definitions.extend(
            McpCapability::new(capability_id, name.clone(), None, tools).tool_definitions(),
        );
    }
    Ok(definitions)
}
pub async fn build_scoped_mcp_tool_definitions(
    servers: &ScopedMcpServers,
    session_id: Option<SessionId>,
    connection_resolver: Option<&Arc<dyn UserConnectionResolver>>,
    egress_service: &dyn EgressService,
) -> Result<Vec<ToolDefinition>> {
    let mut definitions = Vec::new();

    for (name, server) in servers {
        if !server.tool_discovery {
            tracing::debug!(
                server_name = %name,
                "Skipping scoped MCP tool discovery by server config"
            );
            continue;
        }

        let bearer_token =
            match resolve_scoped_mcp_discovery_token(name, server, session_id, connection_resolver)
                .await
            {
                Ok(bearer_token) => bearer_token,
                Err(error) => {
                    tracing::warn!(
                        server_name = %name,
                        error = %error,
                        "Failed to resolve scoped MCP discovery token, skipping server"
                    );
                    continue;
                }
            };
        let has_authorization_header = has_authorization_header(&server.headers);
        if server.auth_mode == McpServerAuthMode::OAuth
            && !has_authorization_header
            && bearer_token.is_none()
        {
            tracing::debug!(
                server_name = %name,
                "Skipping scoped MCP tool discovery because no user connection token is available"
            );
            continue;
        }

        let tools = match fetch_mcp_tools(
            egress_service,
            &server.url,
            bearer_token.as_deref(),
            &server.headers,
        )
        .await
        {
            Ok(tools) => tools,
            Err(error) => {
                tracing::warn!(
                    server_name = %name,
                    error = %error,
                    "Failed to discover scoped MCP tools, skipping server"
                );
                continue;
            }
        };
        let capability_id = session_id
            .map(|id| scoped_mcp_server_uuid(id.uuid(), name))
            .unwrap_or_else(Uuid::nil);
        let capability = McpCapability::new(capability_id, name.clone(), None, tools);
        definitions.extend(capability.tool_definitions());
    }

    Ok(definitions)
}

async fn resolve_scoped_mcp_discovery_token(
    server_name: &str,
    server: &everruns_core::ScopedMcpServer,
    session_id: Option<SessionId>,
    connection_resolver: Option<&Arc<dyn UserConnectionResolver>>,
) -> Result<Option<String>> {
    if server.auth_mode != McpServerAuthMode::OAuth || has_authorization_header(&server.headers) {
        return Ok(None);
    }

    let Some(provider) = server.oauth_provider_id.as_deref() else {
        tracing::debug!(
            server_name,
            "Skipping scoped MCP discovery token lookup because oauth_provider_id is missing"
        );
        return Ok(None);
    };
    let Some(session_id) = session_id else {
        tracing::debug!(
            server_name,
            provider,
            "Skipping scoped MCP discovery token lookup because session_id is unavailable"
        );
        return Ok(None);
    };
    let Some(resolver) = connection_resolver else {
        tracing::debug!(
            server_name,
            provider,
            "Skipping scoped MCP discovery token lookup because connection resolver is unavailable"
        );
        return Ok(None);
    };

    resolver
        .get_connection_token(session_id, provider)
        .await
        .map_err(|error| anyhow!("Failed to resolve scoped MCP discovery token: {error}"))
}

fn has_authorization_header(headers: &HashMap<String, String>) -> bool {
    headers
        .keys()
        .any(|header_name| header_name.eq_ignore_ascii_case("Authorization"))
}

pub fn validate_scoped_mcp_servers(servers: &ScopedMcpServers) -> Result<()> {
    let mut sanitized = HashSet::new();

    for (name, server) in servers {
        if name.trim().is_empty() {
            return Err(anyhow!("Scoped MCP server name cannot be empty"));
        }
        if server.preset.is_some() {
            validate_catalog_reference_shape(name, server)?;
        } else {
            if server.acts_as != McpServerActsAs::None {
                return Err(anyhow!(
                    "Scoped MCP server '{name}' with actsAs '{}' requires a catalog preset for OAuth",
                    server.acts_as
                ));
            }
            // Local-process (stdio) transport is hard-off in the hosted product;
            // it is only available to single-tenant runtime/CLI hosts
            // (knowledge/integrations/runtime-mcp.md D2). Reject it here so it can never be
            // configured on an organization's harness/agent/session.
            if server.transport_type.is_local() {
                return Err(anyhow!(
                    "Scoped MCP server '{name}' uses an unsupported transport: \
                     stdio MCP servers are not allowed in this deployment"
                ));
            }
            validate_safe_url(&server.url)
                .map_err(|e| anyhow!("Invalid scoped MCP server URL for '{name}': {e}"))?;
        }
        let prefix = sanitize_mcp_server_name(name);
        if !everruns_core::mcp_server::is_valid_mcp_server_name(name) {
            return Err(anyhow!(
                "Scoped MCP server name '{name}' is invalid after sanitization: \
                 consecutive or trailing underscores are reserved for MCP tool prefix delimiters"
            ));
        }
        if !sanitized.insert(prefix) {
            return Err(anyhow!(
                "Scoped MCP server names must be unique after sanitization"
            ));
        }
    }

    Ok(())
}

pub fn validate_capability_mcp_servers(servers: &ScopedMcpServers) -> Result<()> {
    for (name, server) in servers {
        if server.preset.is_some() {
            return Err(anyhow!(
                "Capability-contributed MCP server '{name}' cannot use a catalog preset"
            ));
        }
        if server.acts_as != McpServerActsAs::None {
            return Err(anyhow!(
                "Capability-contributed MCP server '{name}' cannot set actsAs"
            ));
        }
    }
    validate_scoped_mcp_servers(servers)
}
fn validate_catalog_reference_shape(name: &str, server: &ScopedMcpServer) -> Result<()> {
    let conflicting_field = if !server.url.is_empty() {
        Some("url")
    } else if !server.headers.is_empty() {
        Some("headers")
    } else if server.command.is_some() {
        Some("command")
    } else if !server.args.is_empty() {
        Some("args")
    } else if !server.env.is_empty() {
        Some("env")
    } else if server.auth_mode != McpServerAuthMode::None {
        Some("auth_mode")
    } else if server.protocol_mode != McpProtocolMode::Auto {
        Some("protocol_mode")
    } else if server.oauth_provider_id.is_some() {
        Some("oauth_provider_id")
    } else if !server.tool_discovery {
        Some("tool_discovery")
    } else {
        None
    };
    if let Some(field) = conflicting_field {
        return Err(anyhow!(
            "Scoped MCP server '{name}' catalog preset reference cannot be combined with inline field '{field}'"
        ));
    }
    Ok(())
}
pub async fn validate_scoped_mcp_servers_for_org(
    db: &StorageBackend,
    org_id: i64,
    servers: &ScopedMcpServers,
) -> Result<()> {
    validate_scoped_mcp_servers(servers)?;
    for (name, server) in servers {
        let Some(preset) = &server.preset else {
            continue;
        };
        let preset_name = preset.catalog_name();
        let row = db
            .get_mcp_server_by_name(org_id, preset_name)
            .await?
            .ok_or_else(|| {
                anyhow!(
                    "Scoped MCP server '{name}' references missing catalog preset '{preset_name}'"
                )
            })?;
        if row.status != "active" {
            return Err(anyhow!(
                "Scoped MCP server '{name}' references catalog preset '{preset_name}' with non-live status '{}'",
                row.status
            ));
        }
        if server.acts_as != McpServerActsAs::None {
            let settings = McpServerService::settings_from_row(&row);
            if settings.auth_mode != McpServerAuthMode::OAuth || settings.oauth.is_none() {
                return Err(anyhow!(
                    "Scoped MCP server '{name}' with actsAs '{}' requires catalog preset '{preset_name}' to have OAuth configuration",
                    server.acts_as
                ));
            }
        }
    }
    Ok(())
}

pub async fn validate_merged_scoped_mcp_servers_for_org<'a, I>(
    db: &StorageBackend,
    org_id: i64,
    layers: I,
) -> Result<ScopedMcpServers>
where
    I: IntoIterator<Item = &'a ScopedMcpServers>,
{
    let merged = merge_scoped_mcp_server_layers(layers);
    validate_scoped_mcp_servers_for_org(db, org_id, &merged).await?;
    Ok(merged)
}

fn scoped_mcp_server_uuid(session_id: Uuid, server_name: &str) -> Uuid {
    Uuid::new_v5(&session_id, server_name.as_bytes())
}

#[cfg(test)]
#[path = "scoped_mcp/cache_tests.rs"]
mod cache_tests;
#[cfg(test)]
#[path = "scoped_mcp/tests.rs"]
mod tests;
