// Browserless Connection Provider
//
// Decision: API-token-based connection (not OAuth). User enters token from Browserless dashboard.
// Decision: Validate token with two probes:
//   1. GET /active — 200/204 means REST works, 401/403 means invalid token.
//   2. GET /chromium — 400/404 means the token passed auth (the path only serves WS
//      upgrades; current Browserless answers a GET with 404), 401/403 means the token
//      lacks CDP access.

use async_trait::async_trait;
use everruns_contracts::connector::{
    Connector, ConnectorFormSchema, ConnectorType, ConnectorValidation, FormField,
};
use tracing::warn;

pub struct BrowserlessConnector;

#[async_trait]
impl Connector for BrowserlessConnector {
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

    fn connection_type(&self) -> ConnectorType {
        ConnectorType::ApiKey
    }

    fn form_schema(&self) -> Option<ConnectorFormSchema> {
        Some(ConnectorFormSchema {
            fields: vec![
                FormField::password("api_key", "API Token")
                    .required()
                    .with_placeholder("your-browserless-api-token"),
            ],
            instructions_markdown: "\
1. Go to [Browserless Dashboard](https://www.browserless.io/account/home)\n\
2. Navigate to **API Keys** in your account settings\n\
3. Copy your API token and paste it below"
                .to_string(),
        })
    }

    async fn validate(&self, credential: &str) -> Result<ConnectorValidation, String> {
        let api_base = crate::browserless::browserless_api_base();
        validate_token(&reqwest::Client::new(), &api_base, credential).await?;
        Ok(ConnectorValidation {
            provider_username: None,
            provider_metadata: None,
        })
    }
}

/// Run both probes against `api_base`. Split out of `validate` so tests can
/// point it at a mock server without mutating process-wide env vars.
async fn validate_token(
    client: &reqwest::Client,
    api_base: &str,
    credential: &str,
) -> Result<(), String> {
    let response = client
        .get(format!("{api_base}/active?token={credential}"))
        .send()
        .await
        .map_err(|e| format!("Failed to reach Browserless API: {e}"))?;

    match response.status().as_u16() {
        // Browserless v2 /active returns 204 No Content; v1 returned 200.
        200 | 204 => {}
        401 | 403 => {
            return Err("Invalid API token. Check that the token is correct and active.".into());
        }
        status => {
            return Err(format!(
                "Unexpected response from Browserless API (HTTP {status})"
            ));
        }
    }

    // Probe the CDP endpoint (/chromium) with a plain HTTP GET. Browserless
    // authenticates the token before routing, so 401/403 still means the token
    // is rejected for CDP sessions even though the path has no GET handler.
    // Decision: a plain GET, not a WebSocket handshake — a completed upgrade
    // launches a billable browser just to check a token.
    let cdp_probe = client
        .get(format!(
            "{api_base}{}?token={credential}",
            crate::browserless::BROWSERLESS_CDP_PATH
        ))
        .send()
        .await;

    match cdp_probe {
        Ok(resp) => {
            let status = resp.status().as_u16();
            match classify_cdp_probe(status) {
                CdpProbe::Accepted => {}
                CdpProbe::Rejected => {
                    warn!(
                        "Browserless token validated for REST but rejected for CDP (HTTP {status})"
                    );
                    return Err(
                        "API token works for REST but does not support CDP/WebSocket sessions. \
                         Check that your Browserless plan includes persistent browser sessions."
                            .into(),
                    );
                }
                CdpProbe::Unexpected => {
                    warn!("Unexpected status from Browserless CDP probe: HTTP {status}");
                }
            }
        }
        Err(e) => {
            warn!("CDP probe failed during validation: {e}");
        }
    }

    Ok(())
}

#[derive(Debug, PartialEq, Eq)]
enum CdpProbe {
    Accepted,
    Rejected,
    Unexpected,
}

fn classify_cdp_probe(status: u16) -> CdpProbe {
    match status {
        // Token accepted; the path only serves WebSocket upgrades. Older
        // Browserless answered a GET with 400; current production answers
        // 404 "No route or file found for resource GET: /chromium" while a
        // WebSocket upgrade on the same path returns 101 (EVERRUNS-2R).
        400 | 404 => CdpProbe::Accepted,
        // The token doesn't have CDP access.
        401 | 403 => CdpProbe::Rejected,
        _ => CdpProbe::Unexpected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_provider_metadata() {
        let p = BrowserlessConnector;
        assert_eq!(p.provider_id(), "browserless");
        assert_eq!(p.display_name(), "Browserless");
        assert_eq!(p.connection_type(), ConnectorType::ApiKey);
        assert_eq!(p.icon(), "browserless");
    }

    #[test]
    fn cdp_probe_404_for_plain_get_means_token_accepted() {
        // Current Browserless answers GET /chromium with 404 for a valid
        // token (no GET route; the WebSocket upgrade returns 101).
        assert_eq!(classify_cdp_probe(404), CdpProbe::Accepted);
        assert_eq!(classify_cdp_probe(400), CdpProbe::Accepted);
        assert_eq!(classify_cdp_probe(401), CdpProbe::Rejected);
        assert_eq!(classify_cdp_probe(403), CdpProbe::Rejected);
        assert_eq!(classify_cdp_probe(500), CdpProbe::Unexpected);
        assert_eq!(classify_cdp_probe(200), CdpProbe::Unexpected);
    }

    async fn mock_browserless(active: u16, chromium: u16) -> wiremock::MockServer {
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/active"))
            .and(query_param("token", "tok"))
            .respond_with(ResponseTemplate::new(active))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/chromium"))
            .and(query_param("token", "tok"))
            .respond_with(ResponseTemplate::new(chromium))
            .mount(&server)
            .await;
        server
    }

    #[tokio::test]
    async fn validate_accepts_current_browserless_404_on_cdp_get() {
        let server = mock_browserless(204, 404).await;
        let result = validate_token(&reqwest::Client::new(), &server.uri(), "tok").await;
        assert_eq!(result, Ok(()));
    }

    #[tokio::test]
    async fn validate_rejects_token_denied_for_cdp() {
        let server = mock_browserless(204, 401).await;
        let err = validate_token(&reqwest::Client::new(), &server.uri(), "tok")
            .await
            .unwrap_err();
        assert!(err.contains("does not support CDP"), "{err}");
    }

    #[tokio::test]
    async fn validate_rejects_invalid_token_before_cdp_probe() {
        let server = mock_browserless(401, 404).await;
        let err = validate_token(&reqwest::Client::new(), &server.uri(), "tok")
            .await
            .unwrap_err();
        assert!(err.contains("Invalid API token"), "{err}");
    }

    #[test]
    fn test_form_schema() {
        let p = BrowserlessConnector;
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
