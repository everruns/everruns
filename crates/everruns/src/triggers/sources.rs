use super::*;
use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
/// Raw GitHub webhook, before parsing. Header values must come from the actual request.
pub struct GitHubRequest<'a> {
    /// X-GitHub-Event.
    pub event: &'a str,
    /// X-GitHub-Delivery.
    pub delivery: &'a str,
    /// X-Hub-Signature-256.
    pub signature: &'a str,
    /// Exact signed bytes.
    pub body: &'a [u8],
}
/// Authenticate and normalize a GitHub delivery. The caller must route only its subscribed repositories/events.
/// GitHub signatures carry no timestamp; retain delivery deduplication and rotate compromised webhook secrets.
pub fn github(
    secret: &str,
    request: GitHubRequest<'_>,
    own_bot: Option<&str>,
) -> Result<Option<Event>, Error> {
    if secret.is_empty() || request.body.len() > 256 * 1024 {
        return Err(Error(
            "Invalid GitHub webhook secret or body exceeds limit".into(),
        ));
    }
    let signature = request
        .signature
        .strip_prefix("sha256=")
        .and_then(|v| hex::decode(v).ok())
        .ok_or_else(|| Error("Invalid GitHub signature".into()))?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .map_err(|_| Error("Invalid GitHub webhook secret".into()))?;
    mac.update(request.body);
    mac.verify_slice(&signature)
        .map_err(|_| Error("Invalid GitHub signature".into()))?;
    let payload: Value = serde_json::from_slice(request.body)
        .map_err(|_| Error("Invalid GitHub webhook JSON".into()))?;
    if own_bot.is_some_and(|bot| {
        payload
            .pointer("/sender/login")
            .and_then(Value::as_str)
            .is_some_and(|login| login.eq_ignore_ascii_case(bot))
    }) {
        return Ok(None);
    }
    let delivery = everruns_core::triggers_github::GitHubDelivery {
        event: request.event.into(),
        delivery_id: request.delivery.into(),
        payload,
    };
    let mut github = everruns_core::triggers_github::github_context(&delivery);
    github["repository_key"] = github["repository"]
        .as_str()
        .map(|r| json!(r.to_ascii_lowercase()))
        .unwrap_or(Value::Null);
    let subject = everruns_core::triggers_github::subject(&github);
    let kind = everruns_core::triggers_github::event_key(
        &delivery.event,
        delivery.payload["action"].as_str(),
    );
    Ok(Some(Event {
        id: delivery.delivery_id,
        source: "github".into(),
        kind,
        subject,
        context: json!({"github":github,"payload":delivery.payload}),
    }))
}
/// Raw MCP event callback, authenticated with Standard Webhooks.
pub struct McpRequest<'a> {
    /// Logical MCP attachment selected by the application.
    pub server: &'a str,
    /// Event name selected by the subscription.
    pub event: &'a str,
    /// Expected live subscription; prevents superseded callbacks.
    pub expected_subscription: &'a str,
    /// Actual x-mcp-subscription-id header.
    pub subscription: &'a str,
    /// Standard Webhooks signing headers.
    pub headers: everruns_integrations::standard_webhooks::SignedHeaders<'a>,
    /// Exact signed body.
    pub body: &'a [u8],
    /// Current Unix seconds, supplied by the host clock.
    pub now: i64,
    /// Optional subject template using payload data.
    pub subject: Option<&'a str>,
}
/// Standard Webhooks header values for MCP callbacks.
pub use everruns_integrations::standard_webhooks::SignedHeaders;
/// Authenticate and normalize an MCP event. Subscription renewal and scheduling belong to the application.
pub fn mcp(secret: &str, request: McpRequest<'_>) -> Result<Event, Error> {
    use everruns_integrations::standard_webhooks as standard;
    if request.expected_subscription.is_empty()
        || request.subscription != request.expected_subscription
    {
        return Err(Error("MCP subscription is not active".into()));
    }
    if request.body.len() > standard::MAX_BODY_BYTES {
        return Err(Error("MCP event exceeds limit".into()));
    }
    let key =
        standard::decode_secret(secret).map_err(|_| Error("Invalid MCP webhook secret".into()))?;
    let id = standard::verify(&key, request.headers, request.body, request.now)
        .map_err(|_| Error("MCP webhook authentication failed".into()))?;
    let body: Value =
        serde_json::from_slice(request.body).map_err(|_| Error("Invalid MCP event JSON".into()))?;
    let context = json!({"mcp":{"server":request.server,"event":request.event,"event_id":body.get("eventId"),"timestamp":body.get("timestamp"),"subscription_id":request.subscription,"delivery_id":id},"payload":body.get("data").cloned().unwrap_or(Value::Null)});
    let subject = request
        .subject
        .map(|template| {
            render_message_template_bounded(template, &context, 512)
                .ok_or_else(|| Error("Rendered MCP subject exceeds limit".into()))
        })
        .transpose()?;
    Ok(Event {
        id: id.into(),
        source: "mcp_event".into(),
        kind: request.event.into(),
        subject,
        context,
    })
}
