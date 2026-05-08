// System email abstraction.
//
// Decision: Keep email delivery as a core, system-wide service rather than an
// agent capability. Email sends are product/ops side effects owned by the host
// application, not tools exposed to agents.
// Decision: Keep provider details behind EmailSender so future SendGrid,
// Cloudflare, SES, or SMTP implementations can reuse the same call sites.
// Decision: Keep the sender fixed until product requirements justify
// per-feature or per-tenant sender identity.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use thiserror::Error;

pub mod resend;

pub use resend::{ResendEmailConfig, ResendEmailSender};

// Intentional current product sender. Keep this as `no-replay`, not `no-reply`,
// until the verified sender identity changes.
pub const SYSTEM_EMAIL_FROM: &str = "no-replay@everruns.com";
const SYSTEM_EMAIL_FROM_NAME: &str = "Everruns";

pub type EmailResult<T> = std::result::Result<T, EmailError>;

#[derive(Debug, Error)]
pub enum EmailError {
    #[error("Email configuration error: {0}")]
    Configuration(String),

    #[error("Invalid email request: {0}")]
    InvalidRequest(String),

    #[error("Email provider transport error: {0}")]
    Transport(String),

    #[error("Email provider error ({provider}, status {status}): {body}")]
    Provider {
        provider: &'static str,
        status: u16,
        body: String,
    },
}

impl EmailError {
    fn config(message: impl Into<String>) -> Self {
        Self::Configuration(message.into())
    }

    fn invalid(message: impl Into<String>) -> Self {
        Self::InvalidRequest(message.into())
    }

    fn transport(error: reqwest::Error) -> Self {
        Self::Transport(error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailAddress {
    pub email: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl EmailAddress {
    pub fn new(email: impl Into<String>) -> Self {
        Self {
            email: email.into(),
            name: None,
        }
    }

    pub fn named(email: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            email: email.into(),
            name: Some(name.into()),
        }
    }

    fn validate(&self, field: &str) -> EmailResult<()> {
        if self.email.trim().is_empty() {
            return Err(EmailError::invalid(format!("{field} email is empty")));
        }
        if self.email.contains(['\r', '\n']) {
            return Err(EmailError::invalid(format!(
                "{field} email contains a newline"
            )));
        }
        if !self.email.contains('@') {
            return Err(EmailError::invalid(format!(
                "{field} email must contain an @ sign"
            )));
        }
        if let Some(name) = &self.name
            && name.contains(['\r', '\n'])
        {
            return Err(EmailError::invalid(format!(
                "{field} name contains a newline"
            )));
        }
        Ok(())
    }

    fn format_for_provider(&self) -> String {
        match self.name.as_deref().filter(|name| !name.trim().is_empty()) {
            Some(name) => format!("{name} <{}>", self.email),
            None => self.email.clone(),
        }
    }
}

impl From<&str> for EmailAddress {
    fn from(email: &str) -> Self {
        Self::new(email)
    }
}

impl From<String> for EmailAddress {
    fn from(email: String) -> Self {
        Self::new(email)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailTag {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EmailTemplate {
    Generic(GenericEmailTemplate),
}

impl EmailTemplate {
    fn render(&self) -> EmailResult<RenderedEmail> {
        match self {
            Self::Generic(template) => template.render(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenericEmailTemplate {
    pub text: String,
    pub html: String,
}

impl GenericEmailTemplate {
    pub fn new(text: impl Into<String>, html: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            html: html.into(),
        }
    }

    fn render(&self) -> EmailResult<RenderedEmail> {
        if self.text.trim().is_empty() {
            return Err(EmailError::invalid("generic email text is required"));
        }
        if self.html.trim().is_empty() {
            return Err(EmailError::invalid("generic email html is required"));
        }

        Ok(RenderedEmail {
            text: format!("Everruns\n\n{}", self.text),
            html: wrap_generic_html(&self.html),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedEmail {
    pub text: String,
    pub html: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailMessage {
    pub to: Vec<EmailAddress>,
    pub subject: String,
    pub template: EmailTemplate,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<EmailTag>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
}

impl EmailMessage {
    pub fn generic(
        to: impl Into<EmailAddress>,
        subject: impl Into<String>,
        text: impl Into<String>,
        html: impl Into<String>,
    ) -> Self {
        Self {
            to: vec![to.into()],
            subject: subject.into(),
            template: EmailTemplate::Generic(GenericEmailTemplate::new(text, html)),
            tags: Vec::new(),
            idempotency_key: None,
        }
    }

    pub fn with_idempotency_key(mut self, key: impl Into<String>) -> Self {
        self.idempotency_key = Some(key.into());
        self
    }

    pub fn with_tag(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.tags.push(EmailTag {
            name: name.into(),
            value: value.into(),
        });
        self
    }

    fn validate(&self) -> EmailResult<RenderedEmail> {
        if self.to.is_empty() {
            return Err(EmailError::invalid("at least one to recipient is required"));
        }
        if self.subject.trim().is_empty() {
            return Err(EmailError::invalid("subject is required"));
        }
        for address in &self.to {
            address.validate("to")?;
        }
        for tag in &self.tags {
            if tag.name.trim().is_empty() {
                return Err(EmailError::invalid("email tag name is required"));
            }
            if tag.value.trim().is_empty() {
                return Err(EmailError::invalid("email tag value is required"));
            }
        }
        if let Some(key) = &self.idempotency_key
            && key.len() > 256
        {
            return Err(EmailError::invalid(
                "idempotency_key must be 256 characters or fewer",
            ));
        }
        if let Some(key) = &self.idempotency_key
            && key.chars().any(|ch| ch.is_ascii_control())
        {
            return Err(EmailError::invalid(
                "idempotency_key must not contain control characters",
            ));
        }
        self.template.render()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SentEmail {
    pub provider: &'static str,
    pub id: String,
}

#[async_trait]
pub trait EmailSender: Send + Sync {
    async fn send_email(&self, message: EmailMessage) -> EmailResult<SentEmail>;

    fn name(&self) -> &'static str {
        "EmailSender"
    }
}

#[derive(Debug, Clone, Default)]
pub struct NoopEmailSender;

#[async_trait]
impl EmailSender for NoopEmailSender {
    async fn send_email(&self, message: EmailMessage) -> EmailResult<SentEmail> {
        message.validate()?;
        Ok(SentEmail {
            provider: "noop",
            id: "noop".to_string(),
        })
    }

    fn name(&self) -> &'static str {
        "NoopEmailSender"
    }
}

#[derive(Debug, Clone, Default)]
pub struct DisabledEmailSender;

#[async_trait]
impl EmailSender for DisabledEmailSender {
    async fn send_email(&self, _message: EmailMessage) -> EmailResult<SentEmail> {
        Err(EmailError::config("system email delivery is disabled"))
    }

    fn name(&self) -> &'static str {
        "DisabledEmailSender"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemEmailConfig {
    Disabled,
    Resend(ResendEmailConfig),
}

impl SystemEmailConfig {
    pub fn from_env() -> EmailResult<Self> {
        let provider = env_opt("EMAIL_PROVIDER").map(|provider| provider.to_ascii_lowercase());
        match provider.as_deref() {
            None | Some("disabled") => Ok(Self::Disabled),
            Some("resend") => ResendEmailConfig::from_env().map(Self::Resend),
            Some(provider) => Err(EmailError::config(format!(
                "unsupported EMAIL_PROVIDER '{provider}'"
            ))),
        }
    }

    pub fn into_sender(self) -> Arc<dyn EmailSender> {
        match self {
            Self::Disabled => Arc::new(DisabledEmailSender),
            Self::Resend(config) => Arc::new(ResendEmailSender::new(config)),
        }
    }
}

pub fn system_email_from() -> EmailAddress {
    EmailAddress::named(SYSTEM_EMAIL_FROM, SYSTEM_EMAIL_FROM_NAME)
}

fn env_opt(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|value| !value.is_empty())
}

fn wrap_generic_html(inner_html: &str) -> String {
    format!(
        r#"<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Everruns</title>
</head>
<body style="margin:0;background:#f6f7f9;color:#111827;font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif;">
  <table role="presentation" width="100%" cellspacing="0" cellpadding="0" style="background:#f6f7f9;padding:32px 16px;">
    <tr>
      <td align="center">
        <table role="presentation" width="100%" cellspacing="0" cellpadding="0" style="max-width:640px;background:#ffffff;border:1px solid #e5e7eb;border-radius:8px;">
          <tr>
            <td style="padding:24px 28px 8px;font-size:18px;font-weight:700;color:#111827;">Everruns</td>
          </tr>
          <tr>
            <td style="padding:12px 28px 28px;font-size:15px;line-height:1.6;color:#1f2937;">{inner_html}</td>
          </tr>
          <tr>
            <td style="padding:18px 28px;border-top:1px solid #e5e7eb;font-size:12px;line-height:1.5;color:#6b7280;">
              This email was sent by Everruns.
            </td>
          </tr>
        </table>
      </td>
    </tr>
  </table>
</body>
</html>"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn generic_template_requires_text_content() {
        let sender = NoopEmailSender;
        let error = sender
            .send_email(EmailMessage::generic(
                "user@example.com",
                "Empty",
                "",
                "<p>Hello</p>",
            ))
            .await
            .unwrap_err();

        assert!(matches!(error, EmailError::InvalidRequest(_)));
        assert!(error.to_string().contains("text"));
    }

    #[tokio::test]
    async fn generic_template_wraps_branding() {
        let message = EmailMessage::generic("user@example.com", "Hi", "Hello", "<p>Hello</p>");
        let rendered = message.validate().unwrap();

        assert!(rendered.text.starts_with("Everruns\n\nHello"));
        assert!(rendered.html.contains("Everruns"));
        assert!(rendered.html.contains("<p>Hello</p>"));
    }

    #[tokio::test]
    async fn disabled_sender_returns_configuration_error() {
        let sender = DisabledEmailSender;
        let error = sender
            .send_email(EmailMessage::generic(
                "user@example.com",
                "Hi",
                "hello",
                "<p>hello</p>",
            ))
            .await
            .unwrap_err();

        assert!(matches!(error, EmailError::Configuration(_)));
        assert!(error.to_string().contains("disabled"));
    }

    #[tokio::test]
    async fn idempotency_key_rejects_control_characters() {
        let sender = NoopEmailSender;
        let error = sender
            .send_email(
                EmailMessage::generic("user@example.com", "Hi", "hello", "<p>hello</p>")
                    .with_idempotency_key("welcome\r\nX-Other: value"),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, EmailError::InvalidRequest(_)));
        assert!(error.to_string().contains("control characters"));
    }
}
