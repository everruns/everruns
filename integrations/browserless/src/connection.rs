// Browserless Connection Provider
//
// Decision: API-token-based connection (not OAuth). User enters token from Browserless dashboard.
// Decision: Validate token with two probes:
//   1. GET /active — 200/204 means REST works, 401/403 means invalid token.
//   2. GET /chromium — 400 means CDP endpoint reachable (expects WS upgrade, not GET),
//      401/403 means token lacks CDP access.

use async_trait::async_trait;
use everruns_core::connection_provider::{
    ConnectionFormSchema, ConnectionProvider, ConnectionType, ConnectionValidation, FieldType,
    FormField,
};
use tracing::warn;

use crate::BROWSERLESS_API_BASE;

pub struct BrowserlessConnectionProvider;

#[async_trait]
impl ConnectionProvider for BrowserlessConnectionProvider {
    fn provider_id(&self) -> &str {
        "browserless"
    }

    fn display_name(&self) -> &str {
        "Browserless"
    }

    fn description(&self) -> &str {
        "Cloud browser automation for screenshots, scraping, and testing"
    }

    fn icon(&self) -> &str {
        "browserless"
    }

    fn connection_type(&self) -> ConnectionType {
        ConnectionType::ApiKey
    }

    fn form_schema(&self) -> Option<ConnectionFormSchema> {
        Some(ConnectionFormSchema {
            fields: vec![FormField {
                name: "api_key".to_string(),
                label: "API Token".to_string(),
                field_type: FieldType::Password,
                required: true,
                placeholder: Some("your-browserless-api-token".to_string()),
                help_text: None,
            }],
            instructions_markdown: "\
1. Go to [Browserless Dashboard](https://www.browserless.io/account/home)\n\
2. Navigate to **API Keys** in your account settings\n\
3. Copy your API token and paste it below"
                .to_string(),
        })
    }

    async fn validate(&self, credential: &str) -> Result<ConnectionValidation, String> {
        let client = reqwest::Client::new();
        let response = client
            .get(format!("{BROWSERLESS_API_BASE}/active?token={credential}"))
            .send()
            .await
            .map_err(|e| format!("Failed to reach Browserless API: {e}"))?;

        match response.status().as_u16() {
            // Browserless v2 /active returns 204 No Content; v1 returned 200.
            200 | 204 => {}
            401 | 403 => {
                return Err(
                    "Invalid API token. Check that the token is correct and active.".into(),
                );
            }
            status => {
                return Err(format!(
                    "Unexpected response from Browserless API (HTTP {status})"
                ));
            }
        }

        // Probe the CDP WebSocket endpoint (/chromium) with a plain HTTP GET.
        // A valid token returns 400 (expects WebSocket upgrade, not HTTP GET).
        // An invalid/expired token for CDP returns 401/403.
        // This catches tokens that work for REST but not for CDP sessions.
        let cdp_probe = client
            .get(format!(
                "{BROWSERLESS_API_BASE}{}?token={credential}",
                crate::BROWSERLESS_CDP_PATH
            ))
            .send()
            .await;

        match cdp_probe {
            Ok(resp) => match resp.status().as_u16() {
                // 400 is the expected success: endpoint expects a WebSocket upgrade, not GET.
                400 => {}
                // 401/403 means the token doesn't have CDP access.
                401 | 403 => {
                    warn!(
                        "Browserless token validated for REST but rejected for CDP (HTTP {})",
                        resp.status().as_u16()
                    );
                    return Err(
                        "API token works for REST but does not support CDP/WebSocket sessions. \
                         Check that your Browserless plan includes persistent browser sessions."
                            .into(),
                    );
                }
                other => {
                    warn!("Unexpected status from Browserless CDP probe: HTTP {other}");
                }
            },
            Err(e) => {
                warn!("CDP probe failed during validation: {e}");
            }
        }

        Ok(ConnectionValidation {
            provider_username: None,
            provider_metadata: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_metadata() {
        let p = BrowserlessConnectionProvider;
        assert_eq!(p.provider_id(), "browserless");
        assert_eq!(p.display_name(), "Browserless");
        assert_eq!(p.connection_type(), ConnectionType::ApiKey);
        assert_eq!(p.icon(), "browserless");
    }

    #[test]
    fn test_form_schema() {
        let p = BrowserlessConnectionProvider;
        let schema = p.form_schema().expect("should have form schema");
        assert_eq!(schema.fields.len(), 1);
        assert_eq!(schema.fields[0].name, "api_key");
        assert!(schema.fields[0].required);
        assert!(
            schema
                .instructions_markdown
                .contains("https://www.browserless.io/account/home")
        );
    }
}
