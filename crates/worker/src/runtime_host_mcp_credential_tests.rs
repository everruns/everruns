//! EVE-1029: the worker path must send the credential the attachment's
//! `actsAs` names, and nothing else.
//!
//! Resolver unit tests can pass while this path sends something different,
//! so these assert on the `Authorization` header that actually reaches the
//! transport, plus which lookup the worker chose.

use super::*;
use crate::worker_adapters::WorkerAdapters;
use everruns_contracts::error::Result as CoreResult;
use everruns_contracts::typed_id::SessionId as CoreSessionId;
use everruns_core::McpServerActsAs;
use everruns_core::connection_services::UserConnectionResolver;
use std::collections::HashMap;
use std::sync::Mutex as StdMutex;

/// Records which lookup the worker used. The legacy lookup and the
/// `actsAs` lookup return distinguishable tokens so a test can tell which
/// one produced the header.
#[derive(Default, Clone)]
struct RecordingResolver {
    acts_as_calls: Arc<StdMutex<Vec<McpServerActsAs>>>,
    legacy_calls: Arc<StdMutex<usize>>,
    execution_inputs: Arc<StdMutex<Vec<Uuid>>>,
    acts_as_token: Option<String>,
    legacy_token: Option<String>,
    fail: bool,
}

#[async_trait::async_trait]
impl UserConnectionResolver for RecordingResolver {
    fn for_execution(&self, input: Uuid) -> Option<Arc<dyn UserConnectionResolver>> {
        self.execution_inputs.lock().unwrap().push(input);
        Some(Arc::new(self.clone()))
    }
    async fn get_connection_token(
        &self,
        _session_id: CoreSessionId,
        _provider: &str,
    ) -> CoreResult<Option<String>> {
        *self.legacy_calls.lock().unwrap() += 1;
        if self.fail {
            return Err(everruns_contracts::error::AgentLoopError::store("db down"));
        }
        Ok(self.legacy_token.clone())
    }

    async fn get_mcp_connection_token(
        &self,
        _session_id: CoreSessionId,
        _provider: &str,
        acts_as: McpServerActsAs,
    ) -> CoreResult<Option<String>> {
        self.acts_as_calls.lock().unwrap().push(acts_as);
        if self.fail {
            return Err(everruns_contracts::error::AgentLoopError::store("db down"));
        }
        Ok(self.acts_as_token.clone())
    }
}

fn server_info(
    acts_as: McpServerActsAs,
    auth_mode: everruns_core::McpServerAuthMode,
    api_key: Option<&str>,
    headers: &[(&str, &str)],
) -> crate::mcp_executor::McpServerInfo {
    crate::mcp_executor::McpServerInfo {
        id: Uuid::new_v4(),
        name: "linear".to_string(),
        url: "https://mcp.linear.app/mcp".to_string(),
        auth_mode,
        protocol_mode: everruns_core::McpProtocolMode::Auto,
        elicitation_policy: Default::default(),
        oauth_provider_id: Some(format!("mcp_oauth_{}", Uuid::new_v4())),
        acts_as,
        api_key: api_key.map(str::to_string),
        headers: headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        secret_bindings: HashMap::new(),
    }
}

/// The header the transport would actually send.
fn authorization_of(connection: &McpConnection) -> Option<String> {
    match &connection.endpoint {
        McpEndpoint::Http { headers, .. } => headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case("authorization"))
            .map(|(_, v)| v.clone()),
        // `McpEndpoint::Stdio` exists only when `everruns-mcp/stdio` is
        // enabled, which a workspace-wide `--all-features` build does. The
        // arm must therefore compile both with and without it, so the
        // wildcard stays and the lint is silenced rather than cfg-gated on
        // a feature this crate does not declare.
        #[allow(unreachable_patterns)]
        _ => None,
    }
}

async fn resolve_with(
    info: crate::mcp_executor::McpServerInfo,
    resolver: RecordingResolver,
) -> (McpConnection, Arc<RecordingResolver>) {
    let resolver = Arc::new(resolver);
    let adapters = StubAdapters {
        info,
        resolver: resolver.clone(),
    };
    let worker_resolver = WorkerMcpResolver {
        input_message_id: None,
        adapters,
        org_id: everruns_core::DEFAULT_ORG_ID,
        session_id: Uuid::new_v4(),
        agent_id: Some(AgentId::from_seed(7)),
    };
    let connection = worker_resolver
        .resolve("linear")
        .await
        .expect("resolve")
        .expect("connection");
    (connection, resolver)
}

fn hosted_resolver(
    info: crate::mcp_executor::McpServerInfo,
    resolver: RecordingResolver,
) -> WorkerMcpResolver<StubAdapters> {
    WorkerMcpResolver {
        input_message_id: None,
        adapters: StubAdapters {
            info,
            resolver: Arc::new(resolver),
        },
        org_id: everruns_core::DEFAULT_ORG_ID,
        session_id: Uuid::new_v4(),
        agent_id: Some(AgentId::from_seed(7)),
    }
}

#[tokio::test]
async fn hosted_mcp_factory_scopes_credentials_to_the_current_input() {
    let input = MessageId::new();
    let resolver = Arc::new(RecordingResolver {
        acts_as_token: Some("current-speaker-token".into()),
        ..Default::default()
    });
    let host = WorkerRuntimeHost::new(StubAdapters {
        info: server_info(
            McpServerActsAs::User,
            everruns_core::McpServerAuthMode::OAuth,
            None,
            &[],
        ),
        resolver: resolver.clone(),
    });
    let hosted = host
        .hosted_mcp_resolver(
            everruns_core::DEFAULT_ORG_ID,
            SessionId::new(),
            Some(AgentId::from_seed(7)),
            input,
        )
        .unwrap();
    let resolved = hosted.resolve("linear").await.unwrap();
    assert_eq!(
        resolved.headers["Authorization"],
        "Bearer current-speaker-token"
    );
    assert_eq!(
        *resolver.execution_inputs.lock().unwrap(),
        vec![input.uuid()]
    );
    assert_eq!(*resolver.legacy_calls.lock().unwrap(), 0);
}

/// EVE-1115: a registered server OpenAI calls gets the same credential
/// the agent's own MCP tools would, and a missing grant fails the turn.
#[tokio::test]
async fn hosted_mcp_resolution_uses_the_same_credentials() {
    use everruns_contracts::hosted_mcp::HostedMcpResolver;
    let oauth = everruns_core::McpServerAuthMode::OAuth;
    let connected = hosted_resolver(
        server_info(McpServerActsAs::User, oauth.clone(), None, &[]),
        RecordingResolver {
            acts_as_token: Some("scoped-token".to_string()),
            ..Default::default()
        },
    );
    let resolved = HostedMcpResolver::resolve(&connected, "linear")
        .await
        .unwrap();
    assert_eq!(resolved.url, "https://mcp.linear.app/mcp");
    assert_eq!(resolved.headers["Authorization"], "Bearer scoped-token");
    assert!(!format!("{resolved:?}").contains("scoped-token"));

    let unconnected = hosted_resolver(
        server_info(McpServerActsAs::User, oauth, None, &[]),
        RecordingResolver::default(),
    );
    let error = HostedMcpResolver::resolve(&unconnected, "linear")
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("/settings/connections"), "{error}");
}

#[tokio::test]
async fn an_acting_identity_routes_through_the_acts_as_lookup_not_the_legacy_one() {
    for acts_as in [McpServerActsAs::Service, McpServerActsAs::User] {
        let (connection, resolver) = resolve_with(
            server_info(acts_as, everruns_core::McpServerAuthMode::OAuth, None, &[]),
            RecordingResolver {
                acts_as_token: Some("scoped-token".to_string()),
                legacy_token: Some("fallback-token".to_string()),
                ..Default::default()
            },
        )
        .await;

        assert_eq!(
            authorization_of(&connection).as_deref(),
            Some("Bearer scoped-token")
        );
        assert_eq!(*resolver.acts_as_calls.lock().unwrap(), vec![acts_as]);
        assert_eq!(
            *resolver.legacy_calls.lock().unwrap(),
            0,
            "{acts_as} must not reach the identity-preferring fallback"
        );
        assert!(connection.pending_oauth_provider.is_none());
    }
}

#[tokio::test]
async fn an_attachment_without_an_acting_identity_never_uses_personal_credentials() {
    // None is an explicit absence of runtime credential authority.
    let (connection, resolver) = resolve_with(
        server_info(
            McpServerActsAs::None,
            everruns_core::McpServerAuthMode::OAuth,
            None,
            &[],
        ),
        RecordingResolver {
            acts_as_token: None,
            legacy_token: Some("fallback-token".to_string()),
            ..Default::default()
        },
    )
    .await;

    assert_eq!(authorization_of(&connection).as_deref(), None);
    assert_eq!(*resolver.legacy_calls.lock().unwrap(), 0);
    assert_eq!(
        *resolver.acts_as_calls.lock().unwrap(),
        vec![McpServerActsAs::None]
    );
}

#[tokio::test]
async fn a_missing_grant_becomes_connection_required_never_an_unauthenticated_call() {
    for acts_as in [McpServerActsAs::Service, McpServerActsAs::User] {
        let (connection, _resolver) = resolve_with(
            server_info(acts_as, everruns_core::McpServerAuthMode::OAuth, None, &[]),
            RecordingResolver {
                acts_as_token: None,
                legacy_token: Some("fallback-token".to_string()),
                ..Default::default()
            },
        )
        .await;

        // No header at all, and the executor is told to prompt rather than
        // let the call go out bare against a permissive server.
        assert_eq!(authorization_of(&connection), None, "{acts_as}");
        let required = connection
            .pending_oauth_provider
            .expect("missing grant must surface connection_required");
        assert_eq!(
            required.subject,
            Some(match acts_as {
                McpServerActsAs::Service => ConnectionRequiredSubject::Agent,
                McpServerActsAs::User => ConnectionRequiredSubject::User,
                McpServerActsAs::None => unreachable!(),
            })
        );
        let expected_setup_url = match acts_as {
            McpServerActsAs::Service => {
                format!("/agents/{}?tab=mcp", AgentId::from_seed(7))
            }
            McpServerActsAs::User => "/settings/connections".to_string(),
            McpServerActsAs::None => unreachable!(),
        };
        assert_eq!(
            required.setup_url.as_deref(),
            Some(expected_setup_url.as_str())
        );
    }
}

#[tokio::test]
async fn an_unscoped_missing_grant_keeps_the_provider_only_shape_without_private_lookup() {
    let (connection, resolver) = resolve_with(
        server_info(
            McpServerActsAs::None,
            everruns_core::McpServerAuthMode::OAuth,
            None,
            &[],
        ),
        RecordingResolver::default(),
    )
    .await;

    assert_eq!(authorization_of(&connection), None);
    assert_eq!(*resolver.legacy_calls.lock().unwrap(), 0);
    assert_eq!(
        *resolver.acts_as_calls.lock().unwrap(),
        vec![McpServerActsAs::None]
    );
    let required = connection
        .pending_oauth_provider
        .expect("unscoped missing grant must still prompt");
    assert_eq!(required.subject, None);
    assert_eq!(required.setup_url, None);
}

#[tokio::test]
async fn an_agentless_service_missing_grant_is_rejected() {
    let resolver = Arc::new(RecordingResolver::default());
    let adapters = StubAdapters {
        info: server_info(
            McpServerActsAs::Service,
            everruns_core::McpServerAuthMode::OAuth,
            None,
            &[],
        ),
        resolver: resolver.clone(),
    };
    let result = WorkerMcpResolver {
        input_message_id: None,
        adapters,
        org_id: everruns_core::DEFAULT_ORG_ID,
        session_id: Uuid::new_v4(),
        agent_id: None,
    }
    .resolve("linear")
    .await;

    assert_eq!(
        result.unwrap_err().to_string(),
        "MCP service attachment requires an agent"
    );
    assert_eq!(
        *resolver.acts_as_calls.lock().unwrap(),
        vec![McpServerActsAs::Service]
    );
}

#[tokio::test]
async fn a_resolver_error_fails_closed_rather_than_calling_unauthenticated() {
    // A DB outage or decryption failure must degrade to a connect prompt,
    // not to a request with no Authorization that a permissive server
    // would happily accept.
    for acts_as in [McpServerActsAs::Service, McpServerActsAs::User] {
        let (connection, resolver) = resolve_with(
            server_info(acts_as, everruns_core::McpServerAuthMode::OAuth, None, &[]),
            RecordingResolver {
                acts_as_token: Some("scoped-token".to_string()),
                legacy_token: Some("fallback-token".to_string()),
                fail: true,
                ..Default::default()
            },
        )
        .await;

        assert_eq!(authorization_of(&connection), None, "{acts_as}");
        assert!(
            connection.pending_oauth_provider.is_some(),
            "{acts_as} must surface connection_required on resolver error"
        );
        // It failed on the acts_as lookup and did not then try the fallback.
        assert_eq!(*resolver.acts_as_calls.lock().unwrap(), vec![acts_as]);
        assert_eq!(*resolver.legacy_calls.lock().unwrap(), 0);
    }
}

#[tokio::test]
async fn a_literal_authorization_header_is_never_overwritten_by_a_resolved_token() {
    // `none` semantics: literal headers are the credential, so resolution
    // must not run at all.
    let (connection, resolver) = resolve_with(
        server_info(
            McpServerActsAs::None,
            everruns_core::McpServerAuthMode::OAuth,
            None,
            &[("Authorization", "Bearer literal-header")],
        ),
        RecordingResolver {
            acts_as_token: Some("scoped-token".to_string()),
            legacy_token: Some("fallback-token".to_string()),
            ..Default::default()
        },
    )
    .await;

    assert_eq!(
        authorization_of(&connection).as_deref(),
        Some("Bearer literal-header")
    );
    assert_eq!(*resolver.legacy_calls.lock().unwrap(), 0);
    assert!(resolver.acts_as_calls.lock().unwrap().is_empty());
}

#[derive(Clone)]
struct StubAdapters {
    info: crate::mcp_executor::McpServerInfo,
    resolver: Arc<RecordingResolver>,
}

#[async_trait::async_trait]
impl WorkerAdapters for StubAdapters {
    async fn get_agent(
        &self,
        _org_id: i64,
        _agent_id: Uuid,
    ) -> CoreResult<Option<everruns_platform::Agent>> {
        unimplemented!()
    }
    async fn get_harness(
        &self,
        _org_id: i64,
        _harness_id: Uuid,
    ) -> CoreResult<Option<everruns_platform::Harness>> {
        unimplemented!()
    }
    async fn get_session(
        &self,
        _org_id: i64,
        _session_id: Uuid,
    ) -> CoreResult<Option<everruns_core::ExecutionSession>> {
        unimplemented!()
    }
    async fn set_session_status(
        &self,
        _org_id: i64,
        _session_id: Uuid,
        _status: &str,
    ) -> CoreResult<()> {
        unimplemented!()
    }
    async fn set_session_title(
        &self,
        _org_id: i64,
        _session_id: Uuid,
        _title: String,
    ) -> CoreResult<everruns_core::ExecutionSession> {
        unimplemented!()
    }
    async fn get_message(
        &self,
        _session_id: Uuid,
        _message_id: Uuid,
    ) -> CoreResult<Option<everruns_core::RuntimeMessage>> {
        unimplemented!()
    }
    async fn load_messages(
        &self,
        _session_id: Uuid,
    ) -> CoreResult<Vec<everruns_core::RuntimeMessage>> {
        unimplemented!()
    }
    async fn emit_event(
        &self,
        _request: everruns_core::events::EventRequest,
    ) -> CoreResult<everruns_core::events::Event> {
        unimplemented!()
    }
    async fn get_model_spec(
        &self,
        _org_id: i64,
        _model_id: Uuid,
    ) -> CoreResult<Option<everruns_contracts::model_spec::ModelSpec>> {
        unimplemented!()
    }
    async fn get_default_model_spec(
        &self,
        _org_id: i64,
    ) -> CoreResult<Option<everruns_contracts::model_spec::ModelSpec>> {
        unimplemented!()
    }
    async fn get_provider_config(
        &self,
        _org_id: i64,
        _provider: &everruns_contracts::runtime_provider::ProviderKey,
    ) -> CoreResult<Option<everruns_contracts::driver_registry::ProviderConfig>> {
        unimplemented!()
    }
    async fn resolve_image(
        &self,
        _org_id: i64,
        _image_id: Uuid,
    ) -> CoreResult<Option<everruns_core::image_services::ResolvedImage>> {
        unimplemented!()
    }
    async fn resolve_images_batch(
        &self,
        _org_id: i64,
        _image_ids: &[Uuid],
    ) -> CoreResult<HashMap<Uuid, everruns_core::image_services::ResolvedImage>> {
        unimplemented!()
    }
    async fn resolve_files_batch(
        &self,
        _org_id: i64,
        _file_ids: &[Uuid],
    ) -> CoreResult<HashMap<Uuid, everruns_core::file_services::ResolvedFile>> {
        unimplemented!()
    }
    async fn read_file(
        &self,
        _org_id: i64,
        _session_id: Uuid,
        _path: &str,
    ) -> CoreResult<Option<everruns_core::session_file::SessionFile>> {
        unimplemented!()
    }
    async fn write_file(
        &self,
        _org_id: i64,
        _session_id: Uuid,
        _path: &str,
        _content: &str,
        _encoding: &str,
    ) -> CoreResult<everruns_core::session_file::SessionFile> {
        unimplemented!()
    }
    async fn delete_file(
        &self,
        _org_id: i64,
        _session_id: Uuid,
        _path: &str,
        _recursive: bool,
    ) -> CoreResult<bool> {
        unimplemented!()
    }
    async fn list_directory(
        &self,
        _org_id: i64,
        _session_id: Uuid,
        _path: &str,
    ) -> CoreResult<Vec<everruns_core::session_file::FileInfo>> {
        unimplemented!()
    }
    async fn stat_file(
        &self,
        _org_id: i64,
        _session_id: Uuid,
        _path: &str,
    ) -> CoreResult<Option<everruns_core::session_file::FileStat>> {
        unimplemented!()
    }
    async fn grep_files(
        &self,
        _org_id: i64,
        _session_id: Uuid,
        _pattern: &str,
        _path_pattern: Option<&str>,
    ) -> CoreResult<Vec<everruns_core::session_file::GrepMatch>> {
        unimplemented!()
    }
    async fn create_directory(
        &self,
        _org_id: i64,
        _session_id: Uuid,
        _path: &str,
    ) -> CoreResult<everruns_core::session_file::FileInfo> {
        unimplemented!()
    }
    async fn get_mcp_server_by_prefix(
        &self,
        _org_id: i64,
        _session_id: Option<Uuid>,
        _server_prefix: &str,
    ) -> CoreResult<crate::mcp_executor::McpServerInfo> {
        Ok(self.info.clone())
    }
    async fn load_turn_context(
        &self,
        _org_id: i64,
        _session_id: Uuid,
    ) -> CoreResult<crate::worker_adapters::TurnContext> {
        unimplemented!()
    }
    async fn invoke_scheduled_channel(
        &self,
        _org_id: i64,
        _app_id: &str,
        _channel_id: &str,
    ) -> CoreResult<serde_json::Value> {
        unimplemented!()
    }
    async fn invoke_agent_trigger(
        &self,
        _org_id: i64,
        _agent_id: &str,
        _trigger_id: &str,
    ) -> CoreResult<serde_json::Value> {
        unimplemented!()
    }
    async fn claim_due_leased_resources(
        &self,
        _limit: u32,
        _stale_after_seconds: u32,
    ) -> CoreResult<Vec<everruns_core::leased_resource::LeasedResource>> {
        unimplemented!()
    }
    async fn mark_leased_resource_released(
        &self,
        _resource_id: everruns_contracts::typed_id::LeasedResourceId,
        _expected_cleanup_started_at: chrono::DateTime<chrono::Utc>,
    ) -> CoreResult<bool> {
        unimplemented!()
    }
    async fn mark_leased_resource_cleanup_failed(
        &self,
        _resource_id: everruns_contracts::typed_id::LeasedResourceId,
        _expected_cleanup_started_at: chrono::DateTime<chrono::Utc>,
        _retry_after_seconds: u32,
        _error: &str,
    ) -> CoreResult<bool> {
        unimplemented!()
    }
    async fn list_orphaned_session_task_ids(
        &self,
        _stale_after: chrono::Duration,
        _limit: i64,
    ) -> CoreResult<Vec<(everruns_contracts::typed_id::SessionId, String)>> {
        unimplemented!()
    }
    async fn prune_terminal_session_tasks(
        &self,
        _ttl: chrono::Duration,
        _limit: i64,
    ) -> CoreResult<usize> {
        unimplemented!()
    }

    fn capability_registry(&self) -> everruns_core::capabilities::CapabilityRegistry {
        unimplemented!()
    }
    fn driver_registry(&self) -> everruns_contracts::DriverRegistry {
        unimplemented!()
    }
    fn sqldb_store(
        &self,
        _org_id: i64,
    ) -> std::sync::Arc<dyn everruns_contracts::session_sqldb::SessionSqlDbStore> {
        unimplemented!()
    }
    fn storage_store(
        &self,
        _org_id: i64,
    ) -> Arc<dyn everruns_core::session_services::SessionStorageStore> {
        unimplemented!()
    }

    fn storage_store_unscoped(
        &self,
    ) -> Arc<dyn everruns_core::session_services::SessionStorageStore> {
        self.storage_store(everruns_core::DEFAULT_ORG_ID)
    }
    fn image_artifact_store(
        &self,
        _org_id: i64,
    ) -> Arc<dyn everruns_core::image_services::ImageArtifactStore> {
        unimplemented!()
    }
    fn provider_credential_store(
        &self,
        _org_id: i64,
    ) -> Arc<dyn everruns_core::connection_services::ProviderCredentialStore> {
        unimplemented!()
    }
    fn utility_llm_service(&self) -> Option<Arc<dyn everruns_core::UtilityLlmService>> {
        unimplemented!()
    }
    fn egress_service(&self) -> Option<Arc<dyn everruns_core::EgressService>> {
        unimplemented!()
    }
    fn platform_store(
        &self,
        _org_id: i64,
        _session_id: everruns_contracts::typed_id::SessionId,
    ) -> Arc<dyn everruns_platform::PlatformStore> {
        unimplemented!()
    }
    fn connection_resolver(
        &self,
    ) -> Arc<dyn everruns_core::connection_services::UserConnectionResolver> {
        self.resolver.clone()
    }
    fn leased_resource_store(
        &self,
    ) -> Arc<dyn everruns_core::session_services::LeasedResourceStore> {
        unimplemented!()
    }
    fn schedule_store(
        &self,
        _org_id: i64,
    ) -> Arc<dyn everruns_core::session_services::SessionScheduleStore> {
        unimplemented!()
    }
    fn reaper_session_task_registry(
        &self,
    ) -> Arc<dyn everruns_core::session_task::SessionTaskRegistry> {
        unimplemented!()
    }
}
