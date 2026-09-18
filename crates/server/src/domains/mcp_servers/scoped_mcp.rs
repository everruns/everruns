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
    cache_context: Option<ScopedMcpCacheContext>,
    connection_resolver: Option<&Arc<dyn UserConnectionResolver>>,
    egress_service: &dyn EgressService,
) -> Result<Vec<ToolDefinition>> {
    let materialized = materialize_scoped_mcp_servers(db, org_id, servers).await?;
    let mut definitions = Vec::new();
    for (name, server) in &materialized {
        let source = servers
            .get(name)
            .expect("materialized server keeps its name");
        let cacheable_identity = !server.acts_as.is_none()
            && source.preset.is_some()
            && session_id.is_some()
            && cache_context.is_some()
            && connection_resolver.is_some();
        if !cacheable_identity {
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
            session_id.expect("cacheable catalog attachment has a session"),
            cache_context.expect("cacheable catalog attachment has cache context"),
            connection_resolver.expect("cacheable catalog attachment has a resolver"),
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
mod tests {
    use super::*;
    use crate::kernel_imports::{HarnessId, ScopedMcpServer, SessionId};
    use crate::storage::models::{CreateMcpServerRow, UpdateMcpServer};
    use chrono::Utc;
    use everruns_platform::{Agent, AgentStatus, generate_agent_public_id};
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct MutableConnectionResolver {
        token: tokio::sync::RwLock<Option<String>>,
    }

    #[async_trait::async_trait]
    impl UserConnectionResolver for MutableConnectionResolver {
        async fn get_connection_token(
            &self,
            _session_id: SessionId,
            _provider: &str,
        ) -> everruns_provider::error::Result<Option<String>> {
            Ok(self.token.read().await.clone())
        }
    }

    #[derive(Default)]
    struct CountingConnectionResolver {
        calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl UserConnectionResolver for CountingConnectionResolver {
        async fn get_connection_token(
            &self,
            _session_id: SessionId,
            _provider: &str,
        ) -> everruns_provider::error::Result<Option<String>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(Some("legacy-token".to_string()))
        }
    }

    #[derive(Default)]
    struct CatalogPreviewEgress {
        calls: AtomicUsize,
    }

    struct IdentityCacheResolver {
        tokens: tokio::sync::RwLock<HashMap<Uuid, Option<String>>>,
    }

    impl IdentityCacheResolver {
        fn new(entries: impl IntoIterator<Item = (SessionId, Option<&'static str>)>) -> Self {
            Self {
                tokens: tokio::sync::RwLock::new(
                    entries
                        .into_iter()
                        .map(|(session_id, token)| (session_id.uuid(), token.map(str::to_string)))
                        .collect(),
                ),
            }
        }

        async fn set(&self, session_id: SessionId, token: Option<&str>) {
            self.tokens
                .write()
                .await
                .insert(session_id.uuid(), token.map(str::to_string));
        }
    }

    #[async_trait::async_trait]
    impl UserConnectionResolver for IdentityCacheResolver {
        async fn get_connection_token(
            &self,
            _session_id: SessionId,
            _provider: &str,
        ) -> everruns_provider::error::Result<Option<String>> {
            panic!("identity-scoped discovery must not use the legacy resolver")
        }

        async fn get_mcp_connection_token(
            &self,
            session_id: SessionId,
            _provider: &str,
            _acts_as: McpServerActsAs,
        ) -> everruns_provider::error::Result<Option<String>> {
            Ok(self
                .tokens
                .read()
                .await
                .get(&session_id.uuid())
                .cloned()
                .flatten())
        }
    }

    struct IdentityCacheEgress {
        calls: AtomicUsize,
        ttl_ms: Option<i64>,
        scope: &'static str,
        delay: Duration,
    }

    impl IdentityCacheEgress {
        fn cacheable(scope: &'static str) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                ttl_ms: Some(60_000),
                scope,
                delay: Duration::ZERO,
            }
        }

        fn tool_name(request: &everruns_core::EgressRequest) -> String {
            request
                .headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                .map(|(_, value)| {
                    format!(
                        "tool_{}",
                        value
                            .strip_prefix("Bearer ")
                            .unwrap_or(value)
                            .replace('-', "_")
                    )
                })
                .unwrap_or_else(|| "tool_anonymous".to_string())
        }
    }

    #[async_trait::async_trait]
    impl EgressService for IdentityCacheEgress {
        async fn send(
            &self,
            request: everruns_core::EgressRequest,
        ) -> everruns_core::EgressResult<everruns_core::EgressResponse> {
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(body["method"], "tools/list");
            self.calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(self.delay).await;
            let mut result = serde_json::json!({
                "tools": [{
                    "name": Self::tool_name(&request),
                    "description": "identity-scoped tool",
                    "inputSchema": {"type": "object"}
                }]
            });
            if let Some(ttl_ms) = self.ttl_ms {
                result["ttlMs"] = ttl_ms.into();
                result["cacheScope"] = self.scope.into();
            }
            Ok(everruns_core::EgressResponse {
                status: 200,
                headers: Default::default(),
                body: serde_json::to_vec(&serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": 1,
                    "result": result
                }))
                .unwrap(),
            })
        }

        async fn send_stream(
            &self,
            _request: everruns_core::EgressRequest,
        ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
            panic!("MCP discovery should not use streaming egress")
        }
    }

    #[async_trait::async_trait]
    impl EgressService for CatalogPreviewEgress {
        async fn send(
            &self,
            request: everruns_core::EgressRequest,
        ) -> everruns_core::EgressResult<everruns_core::EgressResponse> {
            assert!(
                request
                    .headers
                    .keys()
                    .all(|name| !name.eq_ignore_ascii_case("Authorization"))
            );
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            match body["method"].as_str().unwrap_or_default() {
                "initialize" => Ok(everruns_core::EgressResponse {
                    status: 200,
                    headers: std::collections::BTreeMap::from([(
                        "Mcp-Session-Id".to_string(),
                        "preview-session".to_string(),
                    )]),
                    body: serde_json::to_vec(&serde_json::json!({
                        "jsonrpc": "2.0",
                        "id": 0,
                        "result": {
                            "protocolVersion": "2025-06-18",
                            "capabilities": {}
                        }
                    }))
                    .unwrap(),
                }),
                "notifications/initialized" => Ok(everruns_core::EgressResponse {
                    status: 202,
                    headers: Default::default(),
                    body: Vec::new(),
                }),
                "tools/list" => {
                    self.calls.fetch_add(1, Ordering::SeqCst);
                    Ok(everruns_core::EgressResponse {
                        status: 200,
                        headers: Default::default(),
                        body: serde_json::to_vec(&serde_json::json!({
                            "jsonrpc": "2.0",
                            "id": 1,
                            "result": {
                                "tools": [{
                                    "name": "echo",
                                    "description": "Echo a message",
                                    "inputSchema": {"type": "object"}
                                }]
                            }
                        }))
                        .unwrap(),
                    })
                }
                method => panic!("unexpected MCP preview method: {method}"),
            }
        }

        async fn send_stream(
            &self,
            _request: everruns_core::EgressRequest,
        ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
            panic!("MCP discovery should not use streaming egress")
        }
    }

    fn scoped_server(url: &str) -> ScopedMcpServer {
        ScopedMcpServer {
            url: url.to_string(),
            ..Default::default()
        }
    }
    fn oauth_scoped_server(url: &str, provider: &str) -> ScopedMcpServer {
        ScopedMcpServer {
            url: url.to_string(),
            auth_mode: McpServerAuthMode::OAuth,
            oauth_provider_id: Some(provider.to_string()),
            ..Default::default()
        }
    }
    fn catalog_server(preset: &str, acts_as: McpServerActsAs) -> ScopedMcpServer {
        ScopedMcpServer {
            preset: Some(format!("catalog:{preset}").parse().unwrap()),
            acts_as,
            ..Default::default()
        }
    }

    fn materialized_catalog_server(acts_as: McpServerActsAs) -> ScopedMcpServer {
        ScopedMcpServer {
            url: "http://8.8.8.8/mcp".to_string(),
            acts_as,
            ..Default::default()
        }
    }

    async fn discover_for_test(
        db: &StorageBackend,
        preset_id: Uuid,
        acts_as: McpServerActsAs,
        session_id: SessionId,
        context: ScopedMcpCacheContext,
        resolver: &Arc<dyn UserConnectionResolver>,
        egress: &dyn EgressService,
    ) -> Option<Vec<everruns_core::McpToolDefinition>> {
        discover_catalog_tools(
            db,
            everruns_core::DEFAULT_ORG_ID,
            preset_id,
            "linear",
            &materialized_catalog_server(acts_as),
            session_id,
            context,
            resolver,
            egress,
        )
        .await
        .unwrap()
    }
    async fn seed_catalog_server(
        db: &StorageBackend,
        name: &str,
        oauth: bool,
    ) -> everruns_provider::typed_id::McpServerId {
        let settings = crate::domains::mcp_servers::service::McpServerSettings {
            auth_mode: if oauth {
                McpServerAuthMode::OAuth
            } else {
                McpServerAuthMode::None
            },
            protocol_mode: McpProtocolMode::V2025June,
            oauth: oauth
                .then_some(crate::domains::mcp_servers::service::McpServerOAuthSettings::default()),
        };
        db.create_mcp_server(
            everruns_core::DEFAULT_ORG_ID,
            CreateMcpServerRow {
                name: name.to_string(),
                description: None,
                url: "http://8.8.8.8/mcp".to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: Some(serde_json::json!({"X-Catalog":"value"})),
                settings: Some(serde_json::to_value(settings).unwrap()),
            },
        )
        .await
        .unwrap()
        .id
    }

    #[test]
    fn detects_authorization_header_case_insensitively() {
        let mut headers = HashMap::new();
        assert!(!has_authorization_header(&headers));

        headers.insert("authorization".to_string(), "Bearer token".to_string());
        assert!(has_authorization_header(&headers));
    }

    #[test]
    fn cache_keys_keep_acts_as_and_identity_boundaries() {
        let preset_id = Uuid::new_v4();
        let agent_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let context = ScopedMcpCacheContext {
            agent_id: Some(agent_id),
            user_id: Some(user_id),
        };

        assert_eq!(
            cache_identity(
                everruns_core::DEFAULT_ORG_ID,
                preset_id,
                McpServerActsAs::Service,
                context,
            ),
            Some(CacheIdentity::Service {
                org_id: everruns_core::DEFAULT_ORG_ID,
                preset_id,
                agent_id,
            })
        );
        assert_eq!(
            cache_identity(
                everruns_core::DEFAULT_ORG_ID,
                preset_id,
                McpServerActsAs::User,
                context,
            ),
            Some(CacheIdentity::User {
                org_id: everruns_core::DEFAULT_ORG_ID,
                preset_id,
                user_id,
            })
        );
        assert_eq!(
            cache_identity(
                everruns_core::DEFAULT_ORG_ID,
                preset_id,
                McpServerActsAs::None,
                context,
            ),
            None
        );
    }

    #[tokio::test]
    async fn user_cache_never_crosses_users_or_reaches_persistent_storage() {
        let db = StorageBackend::in_memory();
        let preset_id = Uuid::new_v4();
        let agent_id = Uuid::new_v4();
        let user_a = Uuid::new_v4();
        let user_b = Uuid::new_v4();
        let session_a = SessionId::new();
        let session_b = SessionId::new();
        let resolver = Arc::new(IdentityCacheResolver::new([
            (session_a, Some("user-a")),
            (session_b, Some("user-b")),
        ]));
        let resolver_trait: Arc<dyn UserConnectionResolver> = resolver;
        let egress = IdentityCacheEgress::cacheable("public");

        let tools_a = discover_for_test(
            &db,
            preset_id,
            McpServerActsAs::User,
            session_a,
            ScopedMcpCacheContext {
                agent_id: Some(agent_id),
                user_id: Some(user_a),
            },
            &resolver_trait,
            &egress,
        )
        .await
        .unwrap();
        let tools_b = discover_for_test(
            &db,
            preset_id,
            McpServerActsAs::User,
            session_b,
            ScopedMcpCacheContext {
                agent_id: Some(agent_id),
                user_id: Some(user_b),
            },
            &resolver_trait,
            &egress,
        )
        .await
        .unwrap();

        assert_eq!(tools_a[0].name, "tool_user_a");
        assert_eq!(tools_b[0].name, "tool_user_b");
        assert_ne!(tools_a[0].name, tools_b[0].name);
        assert_eq!(egress.calls.load(Ordering::SeqCst), 2);
        assert!(
            db.get_mcp_service_tool_cache(
                everruns_core::DEFAULT_ORG_ID,
                preset_id,
                agent_id,
                "public",
                "",
            )
            .await
            .unwrap()
            .is_none()
        );
    }

    #[tokio::test]
    async fn service_cache_shares_across_users_but_not_agents() {
        let db = StorageBackend::in_memory();
        let preset_id = Uuid::new_v4();
        let agent_a = Uuid::new_v4();
        let agent_b = Uuid::new_v4();
        let session_a = SessionId::new();
        let session_b = SessionId::new();
        let resolver = Arc::new(IdentityCacheResolver::new([
            (session_a, Some("service-token")),
            (session_b, Some("service-token")),
        ]));
        let resolver_trait: Arc<dyn UserConnectionResolver> = resolver;
        let egress = IdentityCacheEgress::cacheable("public");

        for session_id in [session_a, session_b] {
            assert!(
                discover_for_test(
                    &db,
                    preset_id,
                    McpServerActsAs::Service,
                    session_id,
                    ScopedMcpCacheContext {
                        agent_id: Some(agent_a),
                        user_id: Some(Uuid::new_v4()),
                    },
                    &resolver_trait,
                    &egress,
                )
                .await
                .is_some()
            );
        }
        assert_eq!(
            egress.calls.load(Ordering::SeqCst),
            1,
            "two users of one agent share its service grant cache"
        );

        assert!(
            discover_for_test(
                &db,
                preset_id,
                McpServerActsAs::Service,
                session_b,
                ScopedMcpCacheContext {
                    agent_id: Some(agent_b),
                    user_id: Some(Uuid::new_v4()),
                },
                &resolver_trait,
                &egress,
            )
            .await
            .is_some()
        );
        assert_eq!(
            egress.calls.load(Ordering::SeqCst),
            2,
            "different agents never share service cache entries"
        );
    }

    #[tokio::test]
    async fn switching_acts_as_never_reuses_the_old_cache() {
        let db = StorageBackend::in_memory();
        let preset_id = Uuid::new_v4();
        let session_id = SessionId::new();
        let context = ScopedMcpCacheContext {
            agent_id: Some(Uuid::new_v4()),
            user_id: Some(Uuid::new_v4()),
        };
        let resolver = Arc::new(IdentityCacheResolver::new([(
            session_id,
            Some("same-token"),
        )]));
        let resolver_trait: Arc<dyn UserConnectionResolver> = resolver;
        let egress = IdentityCacheEgress::cacheable("public");

        for acts_as in [McpServerActsAs::User, McpServerActsAs::Service] {
            assert!(
                discover_for_test(
                    &db,
                    preset_id,
                    acts_as,
                    session_id,
                    context,
                    &resolver_trait,
                    &egress,
                )
                .await
                .is_some()
            );
        }
        assert_eq!(egress.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn private_service_cache_keeps_the_credential_hash() {
        let db = StorageBackend::in_memory();
        let preset_id = Uuid::new_v4();
        let agent_id = Uuid::new_v4();
        let session_id = SessionId::new();
        let resolver = Arc::new(IdentityCacheResolver::new([(
            session_id,
            Some("credential-one"),
        )]));
        let resolver_trait: Arc<dyn UserConnectionResolver> = resolver.clone();
        let egress = IdentityCacheEgress::cacheable("private");
        let context = ScopedMcpCacheContext {
            agent_id: Some(agent_id),
            user_id: Some(Uuid::new_v4()),
        };

        discover_for_test(
            &db,
            preset_id,
            McpServerActsAs::Service,
            session_id,
            context,
            &resolver_trait,
            &egress,
        )
        .await;
        resolver.set(session_id, Some("credential-two")).await;
        discover_for_test(
            &db,
            preset_id,
            McpServerActsAs::Service,
            session_id,
            context,
            &resolver_trait,
            &egress,
        )
        .await;

        assert_eq!(egress.calls.load(Ordering::SeqCst), 2);
        for token in ["credential-one", "credential-two"] {
            assert!(
                db.get_mcp_service_tool_cache(
                    everruns_core::DEFAULT_ORG_ID,
                    preset_id,
                    agent_id,
                    "private",
                    &credential_hash(token),
                )
                .await
                .unwrap()
                .is_some()
            );
        }
    }

    #[tokio::test]
    async fn public_service_cache_ignores_credential_rotation_for_the_same_agent() {
        let db = StorageBackend::in_memory();
        let preset_id = Uuid::new_v4();
        let session_id = SessionId::new();
        let resolver = Arc::new(IdentityCacheResolver::new([(
            session_id,
            Some("credential-one"),
        )]));
        let resolver_trait: Arc<dyn UserConnectionResolver> = resolver.clone();
        let egress = IdentityCacheEgress::cacheable("public");
        let context = ScopedMcpCacheContext {
            agent_id: Some(Uuid::new_v4()),
            user_id: Some(Uuid::new_v4()),
        };

        let first = discover_for_test(
            &db,
            preset_id,
            McpServerActsAs::Service,
            session_id,
            context,
            &resolver_trait,
            &egress,
        )
        .await
        .unwrap();
        resolver.set(session_id, Some("credential-two")).await;
        let second = discover_for_test(
            &db,
            preset_id,
            McpServerActsAs::Service,
            session_id,
            context,
            &resolver_trait,
            &egress,
        )
        .await
        .unwrap();

        assert_eq!(first[0].name, "tool_credential_one");
        assert_eq!(second[0].name, first[0].name);
        assert_eq!(egress.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn missing_zero_and_negative_ttl_are_never_cached() {
        for ttl_ms in [None, Some(0), Some(-1)] {
            let db = StorageBackend::in_memory();
            let preset_id = Uuid::new_v4();
            let session_id = SessionId::new();
            let resolver = Arc::new(IdentityCacheResolver::new([(
                session_id,
                Some("service-token"),
            )]));
            let resolver_trait: Arc<dyn UserConnectionResolver> = resolver;
            let egress = IdentityCacheEgress {
                calls: AtomicUsize::new(0),
                ttl_ms,
                scope: "public",
                delay: Duration::ZERO,
            };
            let context = ScopedMcpCacheContext {
                agent_id: Some(Uuid::new_v4()),
                user_id: Some(Uuid::new_v4()),
            };

            for _ in 0..2 {
                assert!(
                    discover_for_test(
                        &db,
                        preset_id,
                        McpServerActsAs::Service,
                        session_id,
                        context,
                        &resolver_trait,
                        &egress,
                    )
                    .await
                    .is_some()
                );
            }
            assert_eq!(
                egress.calls.load(Ordering::SeqCst),
                2,
                "ttlMs {ttl_ms:?} must not produce a cache hit"
            );
        }
    }

    #[tokio::test]
    async fn revoked_grant_invalidates_cached_tools() {
        let db = StorageBackend::in_memory();
        let preset_id = Uuid::new_v4();
        let agent_id = Uuid::new_v4();
        let session_id = SessionId::new();
        let resolver = Arc::new(IdentityCacheResolver::new([(
            session_id,
            Some("service-token"),
        )]));
        let resolver_trait: Arc<dyn UserConnectionResolver> = resolver.clone();
        let egress = IdentityCacheEgress::cacheable("public");
        let context = ScopedMcpCacheContext {
            agent_id: Some(agent_id),
            user_id: Some(Uuid::new_v4()),
        };

        assert!(
            discover_for_test(
                &db,
                preset_id,
                McpServerActsAs::Service,
                session_id,
                context,
                &resolver_trait,
                &egress,
            )
            .await
            .is_some()
        );
        resolver.set(session_id, None).await;
        assert!(
            discover_for_test(
                &db,
                preset_id,
                McpServerActsAs::Service,
                session_id,
                context,
                &resolver_trait,
                &egress,
            )
            .await
            .is_none()
        );
        assert!(
            db.get_mcp_service_tool_cache(
                everruns_core::DEFAULT_ORG_ID,
                preset_id,
                agent_id,
                "public",
                "",
            )
            .await
            .unwrap()
            .is_none()
        );

        resolver.set(session_id, Some("service-token")).await;
        assert!(
            discover_for_test(
                &db,
                preset_id,
                McpServerActsAs::Service,
                session_id,
                context,
                &resolver_trait,
                &egress,
            )
            .await
            .is_some()
        );
        assert_eq!(egress.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn cache_past_maximum_age_is_omitted_without_blocking() {
        let db = StorageBackend::in_memory();
        let preset_id = Uuid::new_v4();
        let user_id = Uuid::new_v4();
        let session_id = SessionId::new();
        let identity = CacheIdentity::User {
            org_id: everruns_core::DEFAULT_ORG_ID,
            preset_id,
            user_id,
        };
        USER_TOOL_CACHE.lock().unwrap().insert(
            ScopedToolCacheKey {
                identity,
                scope: CacheScopeKey::Public,
            },
            CachedScopedTools {
                tools: vec![everruns_core::McpToolDefinition {
                    name: "expired".to_string(),
                    description: None,
                    input_schema: serde_json::json!({"type": "object"}),
                    annotations: None,
                }],
                ttl: Duration::from_secs(48 * 60 * 60),
                cached_at: Utc::now() - chrono::Duration::hours(25),
            },
        );
        let resolver = Arc::new(IdentityCacheResolver::new([(
            session_id,
            Some("user-token"),
        )]));
        let resolver_trait: Arc<dyn UserConnectionResolver> = resolver;
        let egress = IdentityCacheEgress::cacheable("public");

        assert!(
            discover_for_test(
                &db,
                preset_id,
                McpServerActsAs::User,
                session_id,
                ScopedMcpCacheContext {
                    agent_id: Some(Uuid::new_v4()),
                    user_id: Some(user_id),
                },
                &resolver_trait,
                &egress,
            )
            .await
            .is_none()
        );
        assert_eq!(egress.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn concurrent_first_fetch_is_single_flight() {
        let db = Arc::new(StorageBackend::in_memory());
        let preset_id = Uuid::new_v4();
        let agent_id = Uuid::new_v4();
        let session_id = SessionId::new();
        let resolver: Arc<dyn UserConnectionResolver> = Arc::new(IdentityCacheResolver::new([(
            session_id,
            Some("service-token"),
        )]));
        let egress = Arc::new(IdentityCacheEgress {
            calls: AtomicUsize::new(0),
            ttl_ms: Some(60_000),
            scope: "private",
            delay: Duration::from_millis(25),
        });
        let context = ScopedMcpCacheContext {
            agent_id: Some(agent_id),
            user_id: Some(Uuid::new_v4()),
        };

        let mut tasks = Vec::new();
        for _ in 0..8 {
            let db = db.clone();
            let resolver = resolver.clone();
            let egress = egress.clone();
            tasks.push(tokio::spawn(async move {
                discover_for_test(
                    db.as_ref(),
                    preset_id,
                    McpServerActsAs::Service,
                    session_id,
                    context,
                    &resolver,
                    egress.as_ref(),
                )
                .await
            }));
        }
        for task in tasks {
            assert!(task.await.unwrap().is_some());
        }
        assert_eq!(egress.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn newly_connected_token_is_visible_on_next_turn_in_same_session() {
        let session_id = SessionId::new();
        let server = oauth_scoped_server("https://mcp.resend.com/mcp", "mcp_oauth_resend");
        let resolver = Arc::new(MutableConnectionResolver {
            token: tokio::sync::RwLock::new(None),
        });
        let resolver_trait: Arc<dyn UserConnectionResolver> = resolver.clone();

        assert!(
            resolve_scoped_mcp_discovery_token(
                "resend",
                &server,
                Some(session_id),
                Some(&resolver_trait),
            )
            .await
            .unwrap()
            .is_none()
        );

        *resolver.token.write().await = Some("fresh-oauth-token".to_string());

        assert_eq!(
            resolve_scoped_mcp_discovery_token(
                "resend",
                &server,
                Some(session_id),
                Some(&resolver_trait),
            )
            .await
            .unwrap()
            .as_deref(),
            Some("fresh-oauth-token")
        );
    }

    fn test_harness() -> Harness {
        Harness {
            id: HarnessId::new(),
            name: "test-harness".to_string(),
            display_name: None,
            icon: None,
            description: None,
            intro_markdown: None,
            short_description: None,
            starters: Vec::new(),
            system_prompt: Some("harness".to_string()),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            network_access: None,
            parallel_tool_calls: None,
            mcp_servers: Default::default(),
            embedder_metadata: Default::default(),
            is_built_in: false,
            status: everruns_platform::HarnessStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            deleted_at: None,
        }
    }

    fn test_agent() -> Agent {
        let public_id = generate_agent_public_id();
        Agent {
            public_id,
            internal_id: public_id.uuid(),
            name: "test-agent".to_string(),
            display_name: None,
            description: None,
            intro_markdown: None,
            short_description: None,
            starters: Vec::new(),
            system_prompt: "agent".to_string(),
            default_model_id: None,
            harness_id: everruns_provider::typed_id::HarnessId::from_uuid(uuid::Uuid::nil()),
            default_version_id: None,
            forked_from_agent_id: None,
            forked_from_version_id: None,
            root_agent_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            tools: vec![],
            mcp_servers: Default::default(),
            status: AgentStatus::Active,
            exposures_suspended: false,
            exposed: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            deleted_at: None,
            usage: None,
        }
    }

    fn test_session(
        harness_id: HarnessId,
        agent_id: everruns_provider::typed_id::AgentId,
    ) -> Session {
        let session_id = SessionId::new();
        Session {
            source: Default::default(),
            activity: Default::default(),
            run_summary: None,
            id: session_id,
            // Default 1:1 session<->workspace: workspace.id mirrors the session id.
            workspace_id: everruns_provider::typed_id::WorkspaceId::from_uuid(session_id.uuid()),
            organization_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
            harness_id,
            agent_id: Some(agent_id),
            agent_version_id: None,
            agent_identity_id: None,
            owner_principal_id: everruns_provider::typed_id::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            owner: None,
            effective_owner: None,
            title: None,
            goal: None,
            locale: None,
            preview: None,
            output_preview: None,
            tags: vec![],
            model_id: None,
            capabilities: vec![],
            tools: vec![],
            mcp_servers: Default::default(),
            system_prompt: None,
            initial_files: vec![],
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            status: everruns_platform::SessionStatus::Started,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            started_at: None,
            finished_at: None,
            usage: None,
            is_pinned: None,
            archived_at: None,
            active_schedule_count: None,
            event_count: None,
            task_count: None,
            file_count: None,
            features: vec![],
            parent_session_id: None,
            forked_from_session_id: None,
            forked_from_sequence: None,
            blueprint_id: None,
            blueprint_config: None,
        }
    }

    #[test]
    fn merge_effective_scoped_mcp_servers_strips_explicit_oauth_fields() {
        let mut harness = test_harness();
        harness.mcp_servers.insert(
            "docs".to_string(),
            oauth_scoped_server("https://harness.example.com/mcp", "github"),
        );

        let agent = test_agent();
        let session = test_session(harness.id, agent.public_id);

        let merged = merge_effective_scoped_mcp_servers(&harness, Some(&agent), &session);
        let docs = merged.get("docs").expect("server exists");

        assert_eq!(docs.auth_mode, McpServerAuthMode::None);
        assert!(docs.oauth_provider_id.is_none());
        assert!(
            !docs.tool_discovery,
            "tool discovery must be disabled on stripped explicit OAuth servers — without a token, an authenticated tools/list would 401",
        );
    }

    #[test]
    fn merge_effective_scoped_mcp_servers_strips_explicit_only_when_oauth() {
        // Non-OAuth explicit entries (e.g. `auth_mode = None`) must pass through
        // unchanged. The sanitizer is narrow on purpose — see the doc comment on
        // strip_untrusted_oauth_from_scoped_mcp_servers.
        let mut harness = test_harness();
        harness.mcp_servers.insert(
            "docs".to_string(),
            scoped_server("https://harness.example.com/mcp"),
        );
        let agent = test_agent();
        let session = test_session(harness.id, agent.public_id);

        let merged = merge_effective_scoped_mcp_servers(&harness, Some(&agent), &session);
        let docs = merged.get("docs").expect("server exists");

        assert_eq!(docs.auth_mode, McpServerAuthMode::None);
        assert!(docs.oauth_provider_id.is_none());
        assert!(docs.tool_discovery);
    }

    #[test]
    fn merge_with_capabilities_preserves_capability_oauth_strips_explicit() {
        use everruns_capability::CapabilityRef as AgentCapabilityConfig;
        use everruns_core::capabilities::{Capability, CapabilityRegistry, RiskLevel};

        struct OAuthMcpCapability;

        impl Capability for OAuthMcpCapability {
            fn id(&self) -> &str {
                "oauth_mcp_test"
            }

            fn name(&self) -> &str {
                "OAuth MCP Test"
            }

            fn description(&self) -> &str {
                "Capability that contributes a scoped MCP server with OAuth"
            }

            fn risk_level(&self) -> RiskLevel {
                RiskLevel::Low
            }

            fn mcp_servers(&self) -> ScopedMcpServers {
                let mut servers = ScopedMcpServers::default();
                servers.insert(
                    "trusted_docs".to_string(),
                    oauth_scoped_server("https://capability.example.com/mcp", "github"),
                );
                servers
            }
        }

        let mut registry = CapabilityRegistry::new();
        registry.register(OAuthMcpCapability);

        let mut harness = test_harness();
        harness
            .capabilities
            .push(AgentCapabilityConfig::new("oauth_mcp_test"));
        // Explicit OAuth entry on a different name — must be sanitized.
        harness.mcp_servers.insert(
            "user_docs".to_string(),
            oauth_scoped_server("https://harness.example.com/mcp", "github"),
        );

        let agent = test_agent();
        let session = test_session(harness.id, agent.public_id);

        let merged = merge_effective_scoped_mcp_servers_with_capabilities(
            &harness,
            Some(&agent),
            &session,
            &registry,
        );

        let trusted = merged.get("trusted_docs").expect("contributed server");
        assert_eq!(
            trusted.auth_mode,
            McpServerAuthMode::OAuth,
            "capability-contributed OAuth must be preserved"
        );
        assert_eq!(trusted.oauth_provider_id.as_deref(), Some("github"));
        assert!(trusted.tool_discovery);

        let user = merged.get("user_docs").expect("explicit server");
        assert_eq!(
            user.auth_mode,
            McpServerAuthMode::None,
            "explicit OAuth must be stripped"
        );
        assert!(user.oauth_provider_id.is_none());
        assert!(!user.tool_discovery);
    }

    #[test]
    fn merge_effective_scoped_mcp_servers_prefers_more_specific_layers() {
        let mut harness = test_harness();
        harness.mcp_servers.insert(
            "docs".to_string(),
            scoped_server("https://harness.example.com/mcp"),
        );

        let mut agent = test_agent();
        agent.mcp_servers.insert(
            "docs".to_string(),
            scoped_server("https://agent.example.com/mcp"),
        );
        agent.mcp_servers.insert(
            "search".to_string(),
            scoped_server("https://agent-search.example.com/mcp"),
        );

        let mut session = test_session(harness.id, agent.public_id);
        session.mcp_servers.insert(
            "docs".to_string(),
            scoped_server("https://session.example.com/mcp"),
        );

        let merged = merge_effective_scoped_mcp_servers(&harness, Some(&agent), &session);

        assert_eq!(merged.len(), 2);
        assert_eq!(
            merged.get("docs").map(|server| server.url.as_str()),
            Some("https://session.example.com/mcp")
        );
        assert_eq!(
            merged.get("search").map(|server| server.url.as_str()),
            Some("https://agent-search.example.com/mcp")
        );
    }

    #[test]
    fn validate_scoped_mcp_servers_rejects_stdio_transport() {
        let mut servers = ScopedMcpServers::default();
        servers.insert(
            "fs".to_string(),
            ScopedMcpServer {
                transport_type: everruns_core::McpServerTransportType::Stdio,
                command: Some("mcp-server-filesystem".to_string()),
                ..Default::default()
            },
        );

        let error = validate_scoped_mcp_servers(&servers).unwrap_err();
        assert!(
            error.to_string().contains("stdio"),
            "expected stdio rejection, got: {error}"
        );
    }

    #[test]
    fn validate_scoped_mcp_servers_rejects_inline_identity_and_preset_fields() {
        for acts_as in [McpServerActsAs::Service, McpServerActsAs::User] {
            let servers = ScopedMcpServers::from([(
                "docs".into(),
                ScopedMcpServer {
                    url: "https://docs.example.com/mcp".into(),
                    acts_as,
                    ..Default::default()
                },
            )]);
            let error = validate_scoped_mcp_servers(&servers).unwrap_err();
            assert!(error.to_string().contains("requires a catalog preset"));
        }

        for (field, server) in [
            (
                "url",
                ScopedMcpServer {
                    url: "https://docs.example.com/mcp".into(),
                    ..catalog_server("linear", McpServerActsAs::None)
                },
            ),
            (
                "headers",
                ScopedMcpServer {
                    headers: HashMap::from([("X-Test".into(), "value".into())]),
                    ..catalog_server("linear", McpServerActsAs::None)
                },
            ),
        ] {
            let servers = ScopedMcpServers::from([("docs".into(), server)]);
            let error = validate_scoped_mcp_servers(&servers).unwrap_err();
            assert!(error.to_string().contains(field), "{error}");
        }
    }

    #[tokio::test]
    async fn catalog_validation_requires_live_existing_oauth_presets() {
        let db = StorageBackend::in_memory();
        let org_id = everruns_core::DEFAULT_ORG_ID;
        let missing = ScopedMcpServers::from([(
            "docs".into(),
            catalog_server("missing", McpServerActsAs::None),
        )]);
        let error = validate_scoped_mcp_servers_for_org(&db, org_id, &missing)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("missing"), "{error}");

        let plain_id = seed_catalog_server(&db, "plain", false).await;
        let needs_oauth = ScopedMcpServers::from([(
            "docs".into(),
            catalog_server("plain", McpServerActsAs::Service),
        )]);
        let error = validate_scoped_mcp_servers_for_org(&db, org_id, &needs_oauth)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("OAuth"), "{error}");
        db.update_mcp_server(
            org_id,
            plain_id.uuid(),
            UpdateMcpServer {
                status: Some("disabled".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let disabled = ScopedMcpServers::from([(
            "docs".into(),
            catalog_server("plain", McpServerActsAs::None),
        )]);
        let error = validate_scoped_mcp_servers_for_org(&db, org_id, &disabled)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("disabled"), "{error}");

        db.update_mcp_server(
            org_id,
            plain_id.uuid(),
            UpdateMcpServer {
                status: Some("archived".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let archived = ScopedMcpServers::from([(
            "docs".into(),
            catalog_server("plain", McpServerActsAs::None),
        )]);
        let error = validate_scoped_mcp_servers_for_org(&db, org_id, &archived)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("plain"), "{error}");

        let deleted_id = seed_catalog_server(&db, "deleted", false).await;
        db.update_mcp_server(
            org_id,
            deleted_id.uuid(),
            UpdateMcpServer {
                status: Some("deleted".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let deleted = ScopedMcpServers::from([(
            "docs".into(),
            catalog_server("deleted", McpServerActsAs::None),
        )]);
        let error = validate_scoped_mcp_servers_for_org(&db, org_id, &deleted)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("deleted"), "{error}");
    }

    #[tokio::test]
    async fn two_logical_names_can_resolve_the_same_catalog_preset() {
        let db = Arc::new(StorageBackend::in_memory());
        let preset_id = seed_catalog_server(&db, "linear", true).await;
        let service = McpServerService::new(db, None);
        let harness = test_harness();
        let agent = test_agent();
        let mut session = test_session(harness.id, agent.public_id);
        session.mcp_servers = ScopedMcpServers::from([
            (
                "issues".into(),
                catalog_server("linear", McpServerActsAs::Service),
            ),
            (
                "projects".into(),
                catalog_server("linear", McpServerActsAs::User),
            ),
        ]);

        let mut descriptor_ids = Vec::new();
        for (prefix, acts_as) in [
            ("issues", McpServerActsAs::Service),
            ("projects", McpServerActsAs::User),
        ] {
            let resolved = resolve_scoped_mcp_server(
                &service,
                everruns_core::DEFAULT_ORG_ID,
                &harness,
                Some(&agent),
                &session,
                prefix,
            )
            .await
            .unwrap()
            .unwrap();
            assert_eq!(resolved.name, prefix);
            assert_eq!(resolved.url, "http://8.8.8.8/mcp");
            assert_eq!(resolved.protocol_mode, McpProtocolMode::V2025June);
            assert_eq!(resolved.acts_as, acts_as);
            assert_eq!(resolved.headers.get("X-Catalog"), Some(&"value".into()));
            // Both attachments declare an acting identity, so both point at the
            // connection store keyed by the shared preset (EVE-1029). The
            // descriptor still carries no credential of its own.
            assert_eq!(resolved.auth_mode, McpServerAuthMode::OAuth);
            assert_eq!(
                resolved.oauth_provider_id.as_deref(),
                Some(everruns_core::mcp_oauth_provider_id_for_uuid(preset_id.uuid()).as_str())
            );
            assert!(resolved.api_key.is_none());
            descriptor_ids.push(resolved.id);
        }

        // Same preset, same provider key, but distinct session-scoped
        // descriptor ids — that is what makes two logical names independent.
        assert_ne!(descriptor_ids[0], descriptor_ids[1]);
    }

    #[tokio::test]
    async fn user_attachment_discards_preset_api_key_and_authorization_header() {
        // A preset carrying service auth, of the shape a config written before
        // validation existed could still have.
        let db = Arc::new(StorageBackend::in_memory());
        let settings = crate::domains::mcp_servers::service::McpServerSettings {
            auth_mode: McpServerAuthMode::OAuth,
            protocol_mode: McpProtocolMode::V2025June,
            oauth: Some(crate::domains::mcp_servers::service::McpServerOAuthSettings::default()),
        };
        db.create_mcp_server(
            everruns_core::DEFAULT_ORG_ID,
            CreateMcpServerRow {
                name: "linear".to_string(),
                description: None,
                url: "http://8.8.8.8/mcp".to_string(),
                transport_type: "http".to_string(),
                api_key_encrypted: None,
                headers: Some(serde_json::json!({
                    "X-Catalog": "value",
                    "Authorization": "Bearer org-service-token",
                })),
                settings: Some(serde_json::to_value(settings).unwrap()),
            },
        )
        .await
        .unwrap();
        let service = McpServerService::new(db, None);
        let harness = test_harness();
        let agent = test_agent();
        let mut session = test_session(harness.id, agent.public_id);
        session.mcp_servers = ScopedMcpServers::from([(
            "projects".into(),
            catalog_server("linear", McpServerActsAs::User),
        )]);

        let resolved = resolve_scoped_mcp_server(
            &service,
            everruns_core::DEFAULT_ORG_ID,
            &harness,
            Some(&agent),
            &session,
            "projects",
        )
        .await
        .unwrap()
        .unwrap();

        // THREAT[TM-TOOL-041]: a `user` attachment can never carry service auth.
        assert!(
            !has_authorization_header(&resolved.headers),
            "org-held Authorization must not survive onto a user attachment"
        );
        assert!(resolved.api_key.is_none());
        // Non-credential headers are untouched, so this is a scrub, not a wipe.
        assert_eq!(resolved.headers.get("X-Catalog"), Some(&"value".into()));
    }

    #[tokio::test]
    async fn none_attachment_keeps_preset_transport_and_literal_headers() {
        let db = Arc::new(StorageBackend::in_memory());
        seed_catalog_server(&db, "linear", false).await;
        let service = McpServerService::new(db, None);
        let harness = test_harness();
        let agent = test_agent();
        let mut session = test_session(harness.id, agent.public_id);
        session.mcp_servers = ScopedMcpServers::from([(
            "docs".into(),
            catalog_server("linear", McpServerActsAs::None),
        )]);

        let resolved = resolve_scoped_mcp_server(
            &service,
            everruns_core::DEFAULT_ORG_ID,
            &harness,
            Some(&agent),
            &session,
            "docs",
        )
        .await
        .unwrap()
        .unwrap();

        // `none` reads no connection store, so it must not be wired to one.
        assert_eq!(resolved.acts_as, McpServerActsAs::None);
        assert_eq!(resolved.auth_mode, McpServerAuthMode::None);
        assert!(resolved.oauth_provider_id.is_none());
        assert!(resolved.api_key.is_none());
        assert_eq!(resolved.headers.get("X-Catalog"), Some(&"value".into()));
    }
    #[tokio::test]
    async fn catalog_preview_discovery_never_uses_legacy_connection_tokens() {
        let db = StorageBackend::in_memory();
        seed_catalog_server(&db, "linear", true).await;
        let resolver = Arc::new(CountingConnectionResolver::default());
        let resolver_trait: Arc<dyn UserConnectionResolver> = resolver.clone();
        let egress = CatalogPreviewEgress::default();

        for acts_as in [
            McpServerActsAs::None,
            McpServerActsAs::Service,
            McpServerActsAs::User,
        ] {
            let servers =
                ScopedMcpServers::from([("docs".to_string(), catalog_server("linear", acts_as))]);
            let materialized =
                materialize_scoped_mcp_servers(&db, everruns_core::DEFAULT_ORG_ID, &servers)
                    .await
                    .unwrap();
            let docs = materialized.get("docs").unwrap();
            assert_eq!(docs.auth_mode, McpServerAuthMode::None);
            assert!(docs.oauth_provider_id.is_none());
            assert_eq!(docs.acts_as, acts_as);
            let discovered = fetch_mcp_tools(&egress, &docs.url, None, &docs.headers)
                .await
                .unwrap();
            assert_eq!(discovered.len(), 1);

            let tools = build_materialized_scoped_mcp_tool_definitions(
                &db,
                everruns_core::DEFAULT_ORG_ID,
                &servers,
                Some(SessionId::new()),
                None,
                Some(&resolver_trait),
                &egress,
            )
            .await
            .unwrap();
            assert_eq!(tools.len(), 1, "catalog preview must expose one MCP tool");
            assert!(tools[0].name().contains("echo"));
        }

        assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
        assert_eq!(egress.calls.load(Ordering::SeqCst), 6);
    }

    #[test]
    fn capability_mcp_validation_rejects_catalog_presets_and_execution_identity() {
        let preset = ScopedMcpServers::from([(
            "docs".to_string(),
            catalog_server("linear", McpServerActsAs::None),
        )]);
        let error = validate_capability_mcp_servers(&preset).unwrap_err();
        assert!(error.to_string().contains("cannot use a catalog preset"));

        let identity = ScopedMcpServers::from([(
            "docs".to_string(),
            ScopedMcpServer {
                url: "https://docs.example.com/mcp".to_string(),
                acts_as: McpServerActsAs::Service,
                ..Default::default()
            },
        )]);
        let error = validate_capability_mcp_servers(&identity).unwrap_err();
        assert!(error.to_string().contains("cannot set actsAs"));
    }

    #[test]
    fn validate_scoped_mcp_servers_rejects_duplicate_sanitized_names() {
        let mut servers = ScopedMcpServers::default();
        servers.insert(
            "Docs API".to_string(),
            scoped_server("https://one.example.com/mcp"),
        );
        servers.insert(
            "docs-api".to_string(),
            scoped_server("https://two.example.com/mcp"),
        );

        let error = validate_scoped_mcp_servers(&servers).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("must be unique after sanitization")
        );
    }

    #[test]
    fn validate_scoped_mcp_servers_rejects_reserved_delimiter_after_sanitization() {
        for name in ["admin__foo", "admin..foo", "admin_", "admin-", "_"] {
            let servers = ScopedMcpServers::from([(
                name.into(),
                scoped_server("https://one.example.com/mcp"),
            )]);
            let error = validate_scoped_mcp_servers(&servers).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("reserved for MCP tool prefix delimiters"),
                "{name}: {error}"
            );
        }
        let servers = ScopedMcpServers::from([(
            "admin_api".into(),
            scoped_server("https://one.example.com/mcp"),
        )]);
        validate_scoped_mcp_servers(&servers).unwrap();
    }

    #[test]
    fn validate_merged_scoped_mcp_servers_rejects_cross_layer_duplicates() {
        let mut harness_servers = ScopedMcpServers::default();
        harness_servers.insert(
            "Docs API".to_string(),
            scoped_server("https://one.example.com/mcp"),
        );

        let mut session_servers = ScopedMcpServers::default();
        session_servers.insert(
            "docs-api".to_string(),
            scoped_server("https://two.example.com/mcp"),
        );

        let error =
            validate_merged_scoped_mcp_servers([&harness_servers, &session_servers]).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("must be unique after sanitization")
        );
    }

    #[test]
    fn scoped_mcp_server_uuid_is_stable_and_namespaced_by_session() {
        let session_a = SessionId::new().uuid();
        let session_b = SessionId::new().uuid();

        assert_eq!(
            scoped_mcp_server_uuid(session_a, "docs"),
            scoped_mcp_server_uuid(session_a, "docs")
        );
        assert_ne!(
            scoped_mcp_server_uuid(session_a, "docs"),
            scoped_mcp_server_uuid(session_b, "docs")
        );
    }
}
