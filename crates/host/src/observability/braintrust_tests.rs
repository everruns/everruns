//! Tests for the Braintrust event listener.

use super::*;
use everruns_core::events::{
    EventContext, EventData, LlmGenerationData, LlmGenerationMetadata, LlmGenerationOutput,
    ReasonCompletedData, ReasonStartedData, TokenUsage, ToolCompletedData, ToolStartedData,
    TurnCompletedData, TurnStartedData,
};
use everruns_core::message::RuntimeMessage;

/// A successful generation that reported nothing but its model.
///
/// Four tests built this same literal inline. They care about event
/// conversion, not metadata, so the shape is noise at the call site and a
/// place to forget a field every time one is added.
fn bare_generation_metadata(model: &str) -> LlmGenerationMetadata {
    LlmGenerationMetadata {
        model: model.to_string(),
        provider: None,
        response_model: None,
        usage: None,
        duration_ms: None,
        time_to_first_token_ms: None,
        success: true,
        error: None,
        finish_reasons: None,
        response_id: None,
        retry: None,
        compaction: None,
        request_options: None,
    }
}
use everruns_provider::tool_types::ToolCall;
use everruns_provider::typed_id::{AgentId, HarnessId, MessageId, SessionId, TurnId};
use serde_json::json;
use tokio::time::sleep;
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn test_config() -> BraintrustConfig {
    BraintrustConfig {
        api_key: "test-api-key".to_string(),
        project_id: "test-project-id".to_string(),
        api_url: "https://api.braintrust.dev".to_string(),
        delivery: BraintrustDeliveryConfig {
            flush_interval: Duration::from_millis(20),
            request_timeout: Duration::from_millis(200),
            ..BraintrustDeliveryConfig::default()
        },
        content: BraintrustContentConfig::default(),
        deployment_grade: DeploymentGrade::Dev,
    }
}

#[path = "braintrust_conversion_tests.rs"]
mod conversion;

#[path = "braintrust_payload_tests.rs"]
mod payload;
