//! Egress fakes `TestServer` wires in place of the network: the MCP servers
//! `mcp_event` triggers subscribe on, and MCP Events callback URLs.
//! Split out of `test_harness.rs` to keep that file under the size guard.

use std::sync::Arc;

use serde_json::Value;

/// Stands in for the MCP servers `mcp_event` triggers subscribe on: requests
/// go to the fake a test installs, and fail while none is.
#[derive(Default)]
pub struct McpServerSlot(pub parking_lot::RwLock<Option<Arc<dyn everruns_core::EgressService>>>);

#[async_trait::async_trait]
impl everruns_core::EgressService for McpServerSlot {
    async fn send(
        &self,
        request: everruns_core::EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressResponse> {
        let server = self.0.read().clone();
        let Some(server) = server else {
            return Err(everruns_core::EgressError::Transport(
                "no MCP server".into(),
            ));
        };
        server.send(request).await
    }

    async fn send_stream(
        &self,
        _request: everruns_core::EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
        Err(everruns_core::EgressError::Transport("no streaming".into()))
    }
}

/// Stands in for MCP Events callback URLs: answers verification challenges
/// (unless told not to) and records every request.
#[derive(Default)]
pub struct WebhookReceiver {
    pub requests: parking_lot::Mutex<Vec<everruns_core::EgressRequest>>,
    pub refuse_verification: std::sync::atomic::AtomicBool,
    /// Statuses to answer event deliveries with, in order; then 200.
    pub delivery_statuses: parking_lot::Mutex<std::collections::VecDeque<u16>>,
}

#[async_trait::async_trait]
impl everruns_core::EgressService for WebhookReceiver {
    async fn send(
        &self,
        request: everruns_core::EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressResponse> {
        let body: Value = serde_json::from_slice(&request.body).unwrap_or_default();
        self.requests.lock().push(request);
        let (status, body) = if body["type"] == "verification" {
            let refuse = self
                .refuse_verification
                .load(std::sync::atomic::Ordering::SeqCst);
            let echoed = if refuse {
                "wrong"
            } else {
                body["challenge"].as_str().unwrap_or("")
            };
            (200, serde_json::json!({ "challenge": echoed }))
        } else {
            let status = self.delivery_statuses.lock().pop_front().unwrap_or(200);
            (status, serde_json::json!({}))
        };
        Ok(everruns_core::EgressResponse {
            status,
            headers: Default::default(),
            body: serde_json::to_vec(&body).unwrap_or_default(),
        })
    }

    async fn send_stream(
        &self,
        _request: everruns_core::EgressRequest,
    ) -> everruns_core::EgressResult<everruns_core::EgressStreamResponse> {
        Err(everruns_core::EgressError::Transport(
            "webhooks do not stream".into(),
        ))
    }
}
