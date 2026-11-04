//! Cloudflare Web Search connection provider: organization search accounts
//! backed by the Cloudflare Web Search API.
//!
//! Decision: validation runs one real one-result search. Nothing cheaper
//! proves all of it at once: that the token has both Workers AI Read and AI
//! Gateway Read, that the gateway exists, and that the index is paid for
//! (credits or the key alias). On Ceramic it costs $0.00025.
//!
//! Decision: the form asks for the account id beside the token, not as a
//! `account:token` pair in one field (Modal's shape), because the account id
//! is not secret: it is stored as `provider_metadata`, so the web search page
//! and the worker can read it without decrypting anything.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::connector::{
    Connector, ConnectorFormSchema, ConnectorType, ConnectorValidation, FormField,
};
use everruns_contracts::runtime::web_search::{
    SearchContext, SearchDriver, SearchRequest, WEB_SEARCH_CONNECTOR_CAPABILITY,
};

use super::CLOUDFLARE_WEB_SEARCH_CONNECTION_PROVIDER;
use super::cloudflare::{CloudflareSearchDriver, CloudflareSearchOptions};
use super::http::DirectEgress;

/// Connection provider for Cloudflare Web Search accounts.
pub struct CloudflareWebSearchConnector;

#[async_trait]
impl Connector for CloudflareWebSearchConnector {
    fn capabilities(&self) -> &'static [&'static str] {
        &[WEB_SEARCH_CONNECTOR_CAPABILITY]
    }

    fn provider_id(&self) -> &str {
        CLOUDFLARE_WEB_SEARCH_CONNECTION_PROVIDER
    }

    fn display_name(&self) -> &str {
        "Cloudflare Web Search"
    }

    fn description(&self) -> &str {
        "Cloudflare Web Search API (Ceramic, Exa or Linkup) as a Web search account"
    }

    fn icon(&self) -> &str {
        "cloud"
    }

    fn connection_type(&self) -> ConnectorType {
        ConnectorType::ApiKey
    }

    fn form_schema(&self) -> Option<ConnectorFormSchema> {
        Some(ConnectorFormSchema {
            fields: vec![
                FormField::text("account_id", "Account ID")
                    .required()
                    .with_placeholder("0123456789abcdef0123456789abcdef")
                    .with_help("32 characters, shown in the dashboard URL and on the account's Overview page."),
                FormField::password("api_key", "API Token")
                    .required()
                    .with_help("Needs Account > Workers AI > Read and Account > AI Gateway > Read."),
                FormField::text("index", "Search index")
                    .with_default("ceramic")
                    .with_placeholder("ceramic")
                    .with_help("ceramic ($0.25 per 1,000, zero data retention), exa ($7) or linkup ($5)."),
                FormField::text("gateway_id", "AI Gateway")
                    .with_default("default")
                    .with_placeholder("default")
                    .with_help("The gateway searches run through; its logs show each search."),
                FormField::text("byok_alias", "Provider key alias")
                    .with_help("Optional. Bill an Exa or Linkup key stored on the gateway instead of AI Gateway credits."),
            ],
            instructions_markdown: "\
One Cloudflare account answers Web search from the index you choose. Searches \
bill to the account's AI Gateway credits at the index's list price, or to a \
provider key stored on the gateway.\n\n\
1. In the [Cloudflare dashboard](https://dash.cloudflare.com/profile/api-tokens), \
create an API token with **Workers AI Read** and **AI Gateway Read**\n\
2. Copy your account ID from the dashboard\n\
3. Paste both below and choose the index\n\n\
Connecting runs one test search."
                .to_string(),
        })
    }

    async fn validate(&self, _credential: &str) -> Result<ConnectorValidation, String> {
        Err("Cloudflare Web Search needs the account ID as well as the API token".to_string())
    }

    async fn validate_fields(
        &self,
        fields: &HashMap<String, String>,
    ) -> Result<ConnectorValidation, String> {
        let field = |name: &str| fields.get(name).map(String::as_str);
        let token = field("api_key").map(str::trim).unwrap_or_default();
        if token.is_empty() {
            return Err("The API token is required".to_string());
        }
        let options = CloudflareSearchOptions::from_fields(
            field("account_id").unwrap_or_default(),
            field("gateway_id"),
            field("index"),
            field("byok_alias"),
        )?;
        let metadata = serde_json::to_value(&options).map_err(|error| error.to_string())?;
        let mut request = SearchRequest::new("Cloudflare");
        request.limit = 1;
        let context = SearchContext {
            egress: Arc::new(DirectEgress::new()),
            network_access: None,
        };
        CloudflareSearchDriver::new(token, options)
            .search(&request, &context)
            .await
            .map_err(|failure| failure.message)?;
        Ok(ConnectorValidation {
            provider_username: None,
            provider_metadata: Some(metadata),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    #[tokio::test]
    async fn bad_fields_fail_before_any_network_call() {
        let connector = CloudflareWebSearchConnector;
        assert!(connector.validate("token").await.is_err());
        let error = connector
            .validate_fields(&fields(&[("account_id", "abc"), ("api_key", "t")]))
            .await
            .unwrap_err();
        assert!(error.contains("32 hexadecimal"), "{error}");
        let error = connector
            .validate_fields(&fields(&[(
                "account_id",
                "0123456789abcdef0123456789abcdef",
            )]))
            .await
            .unwrap_err();
        assert!(error.contains("token"), "{error}");
    }
}
