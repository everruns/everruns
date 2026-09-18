use super::resolve_mcp_connection_token;
use everruns_core::McpServerActsAs;
use everruns_core::connection_services::UserConnectionResolver;
use everruns_provider::error::Result;
use everruns_provider::typed_id::SessionId;
use std::sync::{Arc, Mutex};

struct RecordingResolver {
    calls: Arc<Mutex<Vec<Option<McpServerActsAs>>>>,
}

#[async_trait::async_trait]
impl UserConnectionResolver for RecordingResolver {
    async fn get_connection_token(
        &self,
        _session_id: SessionId,
        _provider: &str,
    ) -> Result<Option<String>> {
        self.calls.lock().unwrap().push(None);
        Ok(Some("legacy".to_string()))
    }

    async fn get_mcp_connection_token(
        &self,
        _session_id: SessionId,
        _provider: &str,
        acts_as: McpServerActsAs,
    ) -> Result<Option<String>> {
        self.calls.lock().unwrap().push(Some(acts_as));
        Ok(Some(acts_as.to_string()))
    }
}

#[tokio::test]
async fn mcp_connection_token_dispatch_uses_only_exact_identity_paths() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let resolver: Arc<dyn UserConnectionResolver> = Arc::new(RecordingResolver {
        calls: calls.clone(),
    });
    let session_id = SessionId::new();

    for (wire_value, expected_token) in [("none", "none"), ("service", "service"), ("user", "user")]
    {
        let token = resolve_mcp_connection_token(&resolver, session_id, "github", wire_value)
            .await
            .unwrap();
        assert_eq!(token.as_deref(), Some(expected_token));
    }

    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            Some(McpServerActsAs::None),
            Some(McpServerActsAs::Service),
            Some(McpServerActsAs::User),
        ]
    );

    let error = resolve_mcp_connection_token(&resolver, session_id, "github", "system")
        .await
        .unwrap_err();
    assert_eq!(error.code(), tonic::Code::InvalidArgument);
    assert_eq!(calls.lock().unwrap().len(), 3);
}
