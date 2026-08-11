// System email abstraction.
//
// Decision (EVE-879): email delivery is a hosted product/ops side effect owned
// by the host application — never a tool exposed to agents and never consumed
// during a turn — so the contract and its concrete senders live in
// `everruns-platform`, not `everruns-core`.
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
        let email = self.email.trim();
        if email.is_empty() {
            return Err(EmailError::invalid(format!("{field} email is empty")));
        }
        if self.email != email {
            return Err(EmailError::invalid(format!(
                "{field} email must not include leading or trailing whitespace"
            )));
        }
        if email.contains(['\r', '\n']) {
            return Err(EmailError::invalid(format!(
                "{field} email contains a newline"
            )));
        }
        if email.contains([' ', '\t', ',', ';', '<', '>', '"', '\'', '(', ')', '[', ']']) {
            return Err(EmailError::invalid(format!(
                "{field} email must be a single mailbox address"
            )));
        }
        let mut parts = email.split('@');
        let local = parts.next().unwrap_or_default();
        let domain = parts.next().unwrap_or_default();
        let has_extra_parts = parts.next().is_some();
        if local.is_empty() || domain.is_empty() || has_extra_parts {
            return Err(EmailError::invalid(format!(
                "{field} email must be a single mailbox address"
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
    Minimal(MinimalEmailTemplate),
    Basic(BasicEmailTemplate),
}

impl EmailTemplate {
    fn render(&self) -> EmailResult<RenderedEmail> {
        match self {
            Self::Minimal(template) => template.render(),
            Self::Basic(template) => template.render(),
        }
    }
}

// Validates the caller-supplied body shared by every template. Each template
// wraps this body differently, but all require non-empty text and HTML.
fn validate_body(text: &str, html: &str) -> EmailResult<()> {
    if text.trim().is_empty() {
        return Err(EmailError::invalid("email text is required"));
    }
    if html.trim().is_empty() {
        return Err(EmailError::invalid("email html is required"));
    }
    Ok(())
}

// Unbranded transactional template styled to match the app (sharp corners,
// grayscale surface, app foreground color). No Everruns wordmark, logo, or
// footer link — use this when the surrounding flow already carries branding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MinimalEmailTemplate {
    pub text: String,
    pub html: String,
}

impl MinimalEmailTemplate {
    pub fn new(text: impl Into<String>, html: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            html: html.into(),
        }
    }

    fn render(&self) -> EmailResult<RenderedEmail> {
        validate_body(&self.text, &self.html)?;
        Ok(RenderedEmail {
            text: self.text.clone(),
            // Neutral title keeps the minimal template free of any branding.
            html: wrap_email_html("Notification", None, &self.html, None),
        })
    }
}

// Branded transactional template. Same app styling as `MinimalEmailTemplate`,
// plus an inline Everruns logo + wordmark header and a footer linking to
// everruns.com.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BasicEmailTemplate {
    pub text: String,
    pub html: String,
}

impl BasicEmailTemplate {
    pub fn new(text: impl Into<String>, html: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            html: html.into(),
        }
    }

    fn render(&self) -> EmailResult<RenderedEmail> {
        validate_body(&self.text, &self.html)?;
        Ok(RenderedEmail {
            text: format!(
                "Everruns\n\n{}\n\n—\nSent by Everruns · {EVERRUNS_SITE_URL}",
                self.text
            ),
            html: wrap_email_html(
                "Everruns",
                Some(&branded_header()),
                &self.html,
                Some(&branded_footer()),
            ),
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
    pub fn minimal(
        to: impl Into<EmailAddress>,
        subject: impl Into<String>,
        text: impl Into<String>,
        html: impl Into<String>,
    ) -> Self {
        Self {
            to: vec![to.into()],
            subject: subject.into(),
            template: EmailTemplate::Minimal(MinimalEmailTemplate::new(text, html)),
            tags: Vec::new(),
            idempotency_key: None,
        }
    }

    pub fn basic(
        to: impl Into<EmailAddress>,
        subject: impl Into<String>,
        text: impl Into<String>,
        html: impl Into<String>,
    ) -> Self {
        Self {
            to: vec![to.into()],
            subject: subject.into(),
            template: EmailTemplate::Basic(BasicEmailTemplate::new(text, html)),
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

// Brand tokens mirrored from apps/ui/src/app/design-system.css. Email clients
// cannot load the Geist webfont or resolve CSS variables, so the app's colors
// and sharp-corner (0px radius) shapes are inlined here as literals. Keep these
// in sync with the design system if the brand palette changes.
const BRAND_NAVY: &str = "#0A1636";
const BRAND_GOLD: &str = "#D4A43A";
const APP_BACKGROUND: &str = "#fafafa";
const APP_SURFACE: &str = "#ffffff";
const APP_BORDER: &str = "#e0e0e0";
const APP_FOREGROUND: &str = "#1a1a1a";
const APP_MUTED_FOREGROUND: &str = "#737373";
const EMAIL_FONT_STACK: &str =
    "-apple-system,BlinkMacSystemFont,'Segoe UI',Roboto,Helvetica,Arial,sans-serif";

// Public Everruns surface used for branded links. Branded email intentionally
// avoids remote images so self-hosted transactional mail does not leak recipient
// open metadata to the upstream marketing domain or its CDN logs.
const EVERRUNS_SITE_URL: &str = "https://everruns.com";

// App-styled HTML shell shared by every template. `title` sets the document
// `<title>` (kept neutral for the unbranded minimal template, since some clients
// surface it in previews). `header` and `footer` are optional pre-rendered table
// rows so branded templates can add a logo header and a footer link while the
// minimal template stays bare.
fn wrap_email_html(
    title: &str,
    header: Option<&str>,
    inner_html: &str,
    footer: Option<&str>,
) -> String {
    let header = header.unwrap_or("");
    let footer = footer.unwrap_or("");
    format!(
        r#"<!doctype html>
<html>
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>{title}</title>
</head>
<body style="margin:0;background:{APP_BACKGROUND};color:{APP_FOREGROUND};font-family:{EMAIL_FONT_STACK};">
  <table role="presentation" width="100%" cellspacing="0" cellpadding="0" style="background:{APP_BACKGROUND};padding:32px 16px;">
    <tr>
      <td align="center">
        <table role="presentation" width="100%" cellspacing="0" cellpadding="0" style="max-width:640px;background:{APP_SURFACE};border:1px solid {APP_BORDER};border-radius:0;">
{header}          <tr>
            <td style="padding:28px;font-size:15px;line-height:1.6;color:{APP_FOREGROUND};">{inner_html}</td>
          </tr>
{footer}        </table>
      </td>
    </tr>
  </table>
</body>
</html>"#
    )
}

// Wordmark header row, linking to everruns.com without loading remote assets.
fn branded_header() -> String {
    format!(
        r#"          <tr>
            <td style="padding:24px 28px;border-bottom:1px solid {APP_BORDER};">
              <table role="presentation" cellspacing="0" cellpadding="0" style="border-collapse:collapse;">
                <tr>
                  <td width="32" style="width:32px;padding:0 10px 0 0;vertical-align:middle;">
                    <a href="{EVERRUNS_SITE_URL}" aria-label="Everruns" style="display:inline-block;text-decoration:none;">
{logo}
                    </a>
                  </td>
                  <td style="vertical-align:middle;">
                    <a href="{EVERRUNS_SITE_URL}" style="text-decoration:none;display:inline-block;font-size:18px;font-weight:700;color:{BRAND_NAVY};letter-spacing:0;">Everruns</a>
                  </td>
                </tr>
              </table>
            </td>
          </tr>
"#,
        logo = branded_logo_svg()
    )
}

// Inline SVG keeps the Basic template branded without fetching a remote image.
// Geometry mirrors knowledge/ui/brand.md: the rings' centroid is centered in the
// viewBox so the mark stays balanced at small email-header sizes.
fn branded_logo_svg() -> String {
    format!(
        r##"                      <svg xmlns="http://www.w3.org/2000/svg" width="28" height="28" viewBox="0 0 512 512" role="img" aria-label="Everruns logo" style="display:block;">
                        <defs>
                          <linearGradient id="emailLogoTop" gradientUnits="userSpaceOnUse" x1="256" y1="63.64" x2="256" y2="256.00">
                            <stop offset="0.00" stop-color="{BRAND_NAVY}"/>
                            <stop offset="0.70" stop-color="{BRAND_NAVY}"/>
                            <stop offset="1.00" stop-color="{BRAND_GOLD}"/>
                          </linearGradient>
                          <linearGradient id="emailLogoLeft" gradientUnits="userSpaceOnUse" x1="89.41" y1="352.18" x2="256" y2="256.00">
                            <stop offset="0.00" stop-color="#081C3F"/>
                            <stop offset="0.70" stop-color="#081C3F"/>
                            <stop offset="1.00" stop-color="{BRAND_GOLD}"/>
                          </linearGradient>
                          <linearGradient id="emailLogoRight" gradientUnits="userSpaceOnUse" x1="422.59" y1="352.18" x2="256" y2="256.00">
                            <stop offset="0.00" stop-color="#0B1233"/>
                            <stop offset="0.70" stop-color="#0B1233"/>
                            <stop offset="1.00" stop-color="{BRAND_GOLD}"/>
                          </linearGradient>
                        </defs>
                        <g fill="none" stroke-width="18" stroke-linecap="round" stroke-linejoin="round">
                          <circle cx="256" cy="183.64" r="120" stroke="url(#emailLogoTop)"/>
                          <circle cx="193.33" cy="292.18" r="120" stroke="url(#emailLogoLeft)"/>
                          <circle cx="318.67" cy="292.18" r="120" stroke="url(#emailLogoRight)"/>
                        </g>
                      </svg>"##
    )
}

// Footer row linking back to everruns.com, with a gold accent link.
fn branded_footer() -> String {
    format!(
        r#"          <tr>
            <td style="padding:18px 28px;border-top:1px solid {APP_BORDER};font-size:12px;line-height:1.5;color:{APP_MUTED_FOREGROUND};">
              Sent by <a href="{EVERRUNS_SITE_URL}" style="color:{BRAND_GOLD};text-decoration:none;font-weight:600;">everruns.com</a>
            </td>
          </tr>
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn minimal_template_requires_text_content() {
        let sender = NoopEmailSender;
        let error = sender
            .send_email(EmailMessage::minimal(
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
    async fn minimal_template_requires_html_content() {
        let sender = NoopEmailSender;
        let error = sender
            .send_email(EmailMessage::minimal(
                "user@example.com",
                "Empty",
                "Hello",
                "  ",
            ))
            .await
            .unwrap_err();

        assert!(matches!(error, EmailError::InvalidRequest(_)));
        assert!(error.to_string().contains("html"));
    }

    #[tokio::test]
    async fn minimal_template_is_app_styled_without_branding() {
        let message = EmailMessage::minimal("user@example.com", "Hi", "Hello", "<p>Hello</p>");
        let rendered = message.validate().unwrap();

        // Body is passed through verbatim — no branding prefix.
        assert_eq!(rendered.text, "Hello");
        assert!(rendered.html.contains("<p>Hello</p>"));
        // App styling: sharp corners and app surface/background colors.
        assert!(rendered.html.contains("border-radius:0"));
        assert!(rendered.html.contains(APP_BACKGROUND));
        // No branding at all in the minimal template — not even the document
        // <title>, which some clients surface in previews.
        assert!(!rendered.html.contains("Everruns"));
        assert!(!rendered.html.contains(EVERRUNS_SITE_URL));
        assert!(!rendered.html.contains("Sent by"));
    }

    #[tokio::test]
    async fn basic_template_adds_branding_without_remote_images() {
        let message = EmailMessage::basic("user@example.com", "Hi", "Hello", "<p>Hello</p>");
        let rendered = message.validate().unwrap();

        // Branded plain text: wordmark prefix and site link footer.
        assert!(rendered.text.starts_with("Everruns\n\nHello"));
        assert!(rendered.text.contains(EVERRUNS_SITE_URL));
        // Branded HTML: shares the app shell, plus logo, wordmark, and link.
        assert!(rendered.html.contains("<p>Hello</p>"));
        assert!(rendered.html.contains("border-radius:0"));
        assert!(rendered.html.contains("Everruns"));
        assert!(rendered.html.contains("<svg"));
        assert!(rendered.html.contains(r#"aria-label="Everruns logo""#));
        assert!(rendered.html.contains("emailLogoTop"));
        assert!(!rendered.html.contains("<img"));
        assert!(
            rendered
                .html
                .contains(&format!(r#"href="{EVERRUNS_SITE_URL}""#))
        );
    }

    #[tokio::test]
    async fn disabled_sender_returns_configuration_error() {
        let sender = DisabledEmailSender;
        let error = sender
            .send_email(EmailMessage::minimal(
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
                EmailMessage::minimal("user@example.com", "Hi", "hello", "<p>hello</p>")
                    .with_idempotency_key("welcome\r\nX-Other: value"),
            )
            .await
            .unwrap_err();

        assert!(matches!(error, EmailError::InvalidRequest(_)));
        assert!(error.to_string().contains("control characters"));
    }

    #[tokio::test]
    async fn rejects_multi_recipient_in_single_to_field() {
        let sender = NoopEmailSender;
        let error = sender
            .send_email(EmailMessage::minimal(
                "victim@example.com, attacker@example.com",
                "Hi",
                "hello",
                "<p>hello</p>",
            ))
            .await
            .unwrap_err();

        assert!(matches!(error, EmailError::InvalidRequest(_)));
        assert!(error.to_string().contains("single mailbox"));
    }

    #[tokio::test]
    async fn rejects_structured_mailbox_syntax_in_raw_email_field() {
        let sender = NoopEmailSender;
        let error = sender
            .send_email(EmailMessage::minimal(
                "Victim <victim@example.com>",
                "Hi",
                "hello",
                "<p>hello</p>",
            ))
            .await
            .unwrap_err();

        assert!(matches!(error, EmailError::InvalidRequest(_)));
        assert!(error.to_string().contains("single mailbox"));
    }

    #[tokio::test]
    async fn rejects_email_with_surrounding_whitespace() {
        let sender = NoopEmailSender;
        let error = sender
            .send_email(EmailMessage::minimal(
                " user@example.com ",
                "Hi",
                "hello",
                "<p>hello</p>",
            ))
            .await
            .unwrap_err();

        assert!(matches!(error, EmailError::InvalidRequest(_)));
        assert!(error.to_string().contains("leading or trailing whitespace"));
    }
}
