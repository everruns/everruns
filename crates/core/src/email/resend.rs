// Resend-backed system email provider.
//
// Decision: Keep this as a concrete EmailSender implementation, not a public
// capability. Resend credentials are process-level deployment configuration.

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::time::Duration;

use super::{
    EmailAddress, EmailError, EmailMessage, EmailResult, EmailSender, EmailTag, RenderedEmail,
    SentEmail, system_email_from,
};

const RESEND_API_BASE_URL: &str = "https://api.resend.com";
const RESEND_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const RESEND_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResendEmailConfig {
    pub api_key: String,
    pub api_base_url: String,
}

impl ResendEmailConfig {
    pub fn from_env() -> EmailResult<Self> {
        let api_key = env_required("RESEND_API_KEY")?;
        let api_base_url = env_opt("RESEND_API_BASE_URL")
            .unwrap_or_else(|| RESEND_API_BASE_URL.trim_end_matches('/').to_string());

        let config = Self {
            api_key,
            api_base_url: api_base_url.trim_end_matches('/').to_string(),
        };
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> EmailResult<()> {
        if self.api_key.trim().is_empty() {
            return Err(EmailError::config("RESEND_API_KEY is empty"));
        }
        let url = reqwest::Url::parse(&self.api_base_url).map_err(|error| {
            EmailError::config(format!("RESEND_API_BASE_URL is not a valid URL: {error}"))
        })?;
        match url.scheme() {
            "http" | "https" => {}
            scheme => {
                return Err(EmailError::config(format!(
                    "RESEND_API_BASE_URL must use http or https, got '{scheme}'"
                )));
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct ResendEmailSender {
    client: Client,
    config: ResendEmailConfig,
}

impl std::fmt::Debug for ResendEmailSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResendEmailSender")
            .field("api_base_url", &self.config.api_base_url)
            .finish_non_exhaustive()
    }
}

impl ResendEmailSender {
    pub fn new(config: ResendEmailConfig) -> Self {
        Self {
            client: Client::builder()
                .connect_timeout(RESEND_CONNECT_TIMEOUT)
                .timeout(RESEND_REQUEST_TIMEOUT)
                .build()
                .expect("build Resend HTTP client"),
            config,
        }
    }

    pub fn config(&self) -> &ResendEmailConfig {
        &self.config
    }
}

#[async_trait]
impl EmailSender for ResendEmailSender {
    async fn send_email(&self, message: EmailMessage) -> EmailResult<SentEmail> {
        let rendered = message.validate()?;
        let request = ResendSendEmailRequest::from_message(system_email_from(), message, rendered);
        let mut builder = self
            .client
            .post(format!("{}/emails", self.config.api_base_url))
            .header("Authorization", format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json");

        if let Some(key) = &request.idempotency_key {
            builder = builder.header("Idempotency-Key", key);
        }

        let response = builder
            .json(&request)
            .send()
            .await
            .map_err(EmailError::transport)?;
        let status = response.status();
        if !status.is_success() {
            return Err(EmailError::Provider {
                provider: "resend",
                status: status.as_u16(),
                body: response.text().await.unwrap_or_default(),
            });
        }

        let body: ResendSendEmailResponse = response.json().await.map_err(EmailError::transport)?;
        Ok(SentEmail {
            provider: "resend",
            id: body.id,
        })
    }

    fn name(&self) -> &'static str {
        "ResendEmailSender"
    }
}

#[derive(Debug, Serialize)]
struct ResendSendEmailRequest {
    from: String,
    to: Vec<String>,
    subject: String,
    html: String,
    text: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tags: Vec<EmailTag>,
    #[serde(skip)]
    idempotency_key: Option<String>,
}

impl ResendSendEmailRequest {
    fn from_message(from: EmailAddress, message: EmailMessage, rendered: RenderedEmail) -> Self {
        Self {
            from: from.format_for_provider(),
            to: message
                .to
                .into_iter()
                .map(|address| address.format_for_provider())
                .collect(),
            subject: message.subject,
            html: rendered.html,
            text: rendered.text,
            tags: message.tags,
            idempotency_key: message.idempotency_key,
        }
    }
}

#[derive(Debug, Deserialize)]
struct ResendSendEmailResponse {
    id: String,
}

fn env_required(name: &str) -> EmailResult<String> {
    env_opt(name).ok_or_else(|| EmailError::config(format!("{name} environment variable not set")))
}

fn env_opt(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn test_config(base_url: String) -> ResendEmailConfig {
        ResendEmailConfig {
            api_key: "re_test".to_string(),
            api_base_url: base_url,
        }
    }

    #[test]
    fn resend_config_rejects_invalid_base_url() {
        let error = ResendEmailConfig {
            api_key: "re_test".to_string(),
            api_base_url: "not a url".to_string(),
        }
        .validate()
        .unwrap_err();

        assert!(matches!(error, EmailError::Configuration(_)));
        assert!(error.to_string().contains("valid URL"));
    }

    #[tokio::test]
    async fn resend_sender_posts_branded_email() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/emails"))
            .and(header("Authorization", "Bearer re_test"))
            .and(header("Idempotency-Key", "welcome/user_123"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "email_123"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let sender = ResendEmailSender::new(test_config(server.uri()));
        let sent = sender
            .send_email(
                EmailMessage::generic("delivered@example.com", "Welcome", "hello", "<p>hello</p>")
                    .with_idempotency_key("welcome/user_123")
                    .with_tag("kind", "welcome"),
            )
            .await
            .unwrap();

        assert_eq!(sent.provider, "resend");
        assert_eq!(sent.id, "email_123");

        let requests = server.received_requests().await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
        assert_eq!(body["from"], "Everruns <no-replay@everruns.com>");
        assert_eq!(body["to"], json!(["delivered@example.com"]));
        assert_eq!(body["subject"], "Welcome");
        assert_eq!(body["text"], "Everruns\n\nhello");
        assert!(body["html"].as_str().unwrap().contains("<p>hello</p>"));
        assert_eq!(
            body["tags"],
            json!([{ "name": "kind", "value": "welcome" }])
        );
        assert!(body.get("cc").is_none());
        assert!(body.get("bcc").is_none());
        assert!(body.get("reply_to").is_none());
        assert!(body.get("headers").is_none());
        assert!(body.get("idempotency_key").is_none());
    }
}
