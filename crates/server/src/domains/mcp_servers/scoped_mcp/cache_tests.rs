use super::*;
use crate::kernel_imports::SessionId;
use std::sync::atomic::{AtomicUsize, Ordering};

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
