//! Bind the shared Chat Completions driver to the application's host HTTP.
use everruns_contracts::http_transport::HttpTransport;
use everruns_contracts::{BearerAuth, ChatDriver, OpenAIProtocolChatDriver, Provider};
use std::sync::Arc;

pub fn driver(
    base_url: String,
    api_key: String,
    transport: Arc<dyn HttpTransport>,
) -> Arc<dyn ChatDriver> {
    Arc::from(
        Provider::new(
            "model",
            OpenAIProtocolChatDriver::new().with_transport(transport),
        )
        .base_url(base_url)
        .auth(BearerAuth::new(api_key))
        .into_boxed_driver(),
    )
}
