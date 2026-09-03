// URL mode elicitation for Everruns' own MCP server endpoint
// (spec: knowledge/integrations/mcp.md, "URL mode elicitation").
//
// Some things an MCP client must never be asked for. A session secret and a
// third-party connection are both credentials: routed through `tools/call`
// arguments they would pass through the client, the model's context, and the
// event log. URL mode elicitation is the protocol's answer — the server asks a
// human to complete the interaction out of band, on a page the server renders
// itself, and the client only ever sees a URL.
//
// Two decisions shape this module:
//
// 1. **Delivered as MRTR, never as a server-initiated request.** `/mcp` is
//    stateless request/response with no server→client stream, so an
//    `elicitation/create` request could not be delivered even if we wanted to.
//    `2026-07-28` requires MRTR anyway: the elicitation rides in an
//    `input_required` result and the client retries the call. (The `-32042`
//    URLElicitationRequiredError from SEP-1036 existed only in `2025-11-25`
//    and this version forbids emitting it.)
//
// 2. **State lives in a signed token, not in a table.** The endpoint stores no
//    per-request state, so the intent ("collect secret X for session Y, on
//    behalf of user Z") is HMAC-signed and travels in both the elicitation URL
//    and `requestState`. Per MRTR, `requestState` is attacker-controlled input:
//    it is verified on every use, bound to the authenticated principal, and
//    short-lived. The token is *not* a credential — the page it names still
//    requires the user's own browser session, and refuses to act when that
//    session belongs to a different user. That check is what defeats the
//    phishing attack the elicitation spec describes, where one user hands their
//    elicitation URL to another.

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::Sha256;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// Per-request client-capabilities `_meta` key.
const CLIENT_CAPABILITIES_META_KEY: &str = "io.modelcontextprotocol/clientCapabilities";

/// `MissingRequiredClientCapability` (2026-07-28). Returned when a request can
/// only be served by eliciting and the client never declared it can be
/// elicited.
pub(super) const MISSING_CAPABILITY_ERROR_CODE: i32 = -32021;

/// How long an elicitation token stays valid. Long enough to finish an OAuth
/// flow or paste a key, short enough to bound replay of a leaked URL.
const TOKEN_TTL_SECONDS: i64 = 900;

/// Path segment under the API root that serves elicitation pages.
pub const ELICITATION_PATH: &str = "/mcp/elicitations";

/// What a human is being sent off to do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ElicitationIntent {
    /// Store an encrypted session secret, typed into a server-rendered form.
    SessionSecret { session_id: String, name: String },
    /// Authorize a third-party provider connection for this user.
    Connect { provider: String },
}

impl ElicitationIntent {
    /// Page that serves this intent, relative to the API root.
    fn page(&self) -> &'static str {
        match self {
            Self::SessionSecret { .. } => "secret",
            Self::Connect { .. } => "connect",
        }
    }

    /// Message shown to the user by the *client*, before they consent to
    /// opening the URL. Names what will happen and where.
    pub fn message(&self) -> String {
        match self {
            Self::SessionSecret { session_id, name } => format!(
                "Everruns needs the value of '{name}' for session {session_id}. \
                 Open the secure form to enter it — the value is encrypted at rest \
                 and is never sent through this MCP client or the model."
            ),
            Self::Connect { provider } => format!(
                "Everruns needs your authorization to act on your behalf in '{provider}'. \
                 Open the connection page to sign in — the credentials stay with Everruns \
                 and are never sent through this MCP client."
            ),
        }
    }

    /// Key this elicitation is delivered under in `inputRequests`. Stable per
    /// intent kind so a client retrying sees a coherent key.
    pub fn request_key(&self) -> &'static str {
        self.page()
    }
}

/// The signed intent, bound to a principal and an expiry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ElicitationToken {
    /// Authenticated principal that started the elicitation. The page refuses
    /// to act for anyone else, and `requestState` presented by another
    /// principal is rejected.
    pub user_id: Uuid,
    /// Organization the call resolved to.
    pub org_id: String,
    pub intent: ElicitationIntent,
    /// Unix seconds after which the token is dead.
    pub expires_at: i64,
    /// Makes each token unique so two identical intents do not collide.
    pub nonce: String,
}

impl ElicitationToken {
    pub fn new(
        user_id: Uuid,
        org_id: impl Into<String>,
        intent: ElicitationIntent,
        now: i64,
    ) -> Self {
        Self {
            user_id,
            org_id: org_id.into(),
            intent,
            expires_at: now + TOKEN_TTL_SECONDS,
            nonce: Uuid::new_v4().to_string(),
        }
    }
}

/// Why a presented token was refused. Callers map these to a JSON-RPC error or
/// an HTTP status; none of them leak why beyond "this is not usable".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenError {
    Malformed,
    BadSignature,
    Expired,
    /// Presented by a principal other than the one it was minted for.
    WrongPrincipal,
}

impl TokenError {
    pub fn message(self) -> &'static str {
        match self {
            Self::Expired => "This link has expired. Run the tool again to get a fresh one.",
            Self::WrongPrincipal => {
                "This link was created for a different user. Ask for your own link \
                 rather than opening one someone sent you."
            }
            Self::Malformed | Self::BadSignature => "This link is not valid.",
        }
    }
}

/// Sign a token into the opaque `payload.signature` string used as both the
/// URL parameter and `requestState`.
pub fn sign_token(token: &ElicitationToken, secret: &str) -> String {
    let payload = serde_json::to_vec(token).expect("elicitation token serializes");
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any length");
    mac.update(&payload);
    format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(&payload),
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
    )
}

/// Verify integrity, expiry, and principal binding, in that order.
///
/// `presented_by` is the principal making the *current* request — the MCP
/// caller on a retry, or the browser session on the page. Rejecting a mismatch
/// is what stops one user's elicitation from being completed by another.
pub fn verify_token(
    value: &str,
    secret: &str,
    presented_by: Uuid,
    now: i64,
) -> Result<ElicitationToken, TokenError> {
    let (payload, signature) = value.split_once('.').ok_or(TokenError::Malformed)?;
    let payload = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| TokenError::Malformed)?;
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| TokenError::Malformed)?;
    let mut mac =
        HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts keys of any length");
    mac.update(&payload);
    mac.verify_slice(&signature)
        .map_err(|_| TokenError::BadSignature)?;
    let token: ElicitationToken =
        serde_json::from_slice(&payload).map_err(|_| TokenError::Malformed)?;
    if token.expires_at <= now {
        return Err(TokenError::Expired);
    }
    if token.user_id != presented_by {
        return Err(TokenError::WrongPrincipal);
    }
    Ok(token)
}

/// Absolute URL of the page that serves an intent, carrying its signed token.
///
/// Always https in a deployment (`base_url` is the configured public root), and
/// carries no secret material: the token names *what* to collect, never a
/// value, and grants no access on its own.
pub fn elicitation_url(base_url: &str, intent: &ElicitationIntent, signed: &str) -> String {
    format!(
        "{}{ELICITATION_PATH}/{}?token={}",
        base_url.trim_end_matches('/'),
        intent.page(),
        urlencoding_encode(signed)
    )
}

/// Percent-encode a token for a query string. The signed form is base64url
/// plus a dot, so only `.` and the base64url alphabet can appear; encoding is
/// belt-and-braces against a future format change.
fn urlencoding_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

/// True when this request's `_meta` declares URL mode elicitation.
///
/// Both the explicit `elicitation.url` shape and — deliberately — nothing else:
/// an empty `elicitation` object means form mode only, which must never receive
/// a URL.
pub(super) fn client_supports_url_elicitation(params: &Value) -> bool {
    params
        .get("_meta")
        .and_then(|meta| meta.get(CLIENT_CAPABILITIES_META_KEY))
        .and_then(|caps| caps.get("elicitation"))
        .and_then(|elicitation| elicitation.get("url"))
        .is_some()
}

/// Build the `InputRequiredResult` that carries one URL mode elicitation.
pub(super) fn url_elicitation_result(intent: &ElicitationIntent, url: &str, signed: &str) -> Value {
    json!({
        "resultType": "input_required",
        "inputRequests": {
            intent.request_key(): {
                "method": "elicitation/create",
                "params": {
                    "mode": "url",
                    "url": url,
                    "message": intent.message(),
                }
            }
        },
        "requestState": signed,
    })
}

/// `data` payload for a `MissingRequiredClientCapability` error, naming what
/// the client would have to declare.
pub(super) fn missing_capability_data() -> Value {
    json!({
        "requiredCapabilities": { "elicitation": { "url": {} } }
    })
}

/// The `requestState` a client echoed back, if any.
pub(super) fn request_state(params: &Value) -> Option<&str> {
    params.get("requestState").and_then(Value::as_str)
}

/// Whether the client accepted the elicitation it was handed under `key`.
///
/// A `decline` or `cancel` is a real answer, not an error: the caller reports
/// it as a tool result so the model can tell the user nothing was stored.
pub(super) fn accepted(params: &Value, key: &str) -> Option<bool> {
    let action = params
        .get("inputResponses")?
        .get(key)?
        .get("action")?
        .as_str()?;
    Some(action == "accept")
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "test-signing-secret";
    const NOW: i64 = 1_800_000_000;

    fn intent() -> ElicitationIntent {
        ElicitationIntent::SessionSecret {
            session_id: "sess_1".to_string(),
            name: "OPENAI_API_KEY".to_string(),
        }
    }

    #[test]
    fn round_trips_a_token_for_its_own_principal() {
        let user = Uuid::new_v4();
        let token = ElicitationToken::new(user, "org_1", intent(), NOW);
        let signed = sign_token(&token, SECRET);
        assert_eq!(verify_token(&signed, SECRET, user, NOW).unwrap(), token);
    }

    #[test]
    fn refuses_tampering_expiry_and_another_principal() {
        let user = Uuid::new_v4();
        let signed = sign_token(&ElicitationToken::new(user, "org_1", intent(), NOW), SECRET);

        // A different signing key, i.e. a forged or replayed-from-elsewhere state.
        assert_eq!(
            verify_token(&signed, "other-secret", user, NOW),
            Err(TokenError::BadSignature)
        );
        // Payload edited in place: the signature no longer covers it.
        let (payload, signature) = signed.split_once('.').unwrap();
        let mut decoded: Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).unwrap()).unwrap();
        decoded["intent"]["name"] = json!("STOLEN");
        let tampered = format!(
            "{}.{signature}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&decoded).unwrap())
        );
        assert_eq!(
            verify_token(&tampered, SECRET, user, NOW),
            Err(TokenError::BadSignature)
        );
        // Past its TTL.
        assert_eq!(
            verify_token(&signed, SECRET, user, NOW + TOKEN_TTL_SECONDS + 1),
            Err(TokenError::Expired)
        );
        // The phishing case: someone else opened the link.
        assert_eq!(
            verify_token(&signed, SECRET, Uuid::new_v4(), NOW),
            Err(TokenError::WrongPrincipal)
        );
        assert_eq!(
            verify_token("not-a-token", SECRET, user, NOW),
            Err(TokenError::Malformed)
        );
    }

    #[test]
    fn only_an_explicit_url_mode_declaration_counts() {
        let declared = json!({ "_meta": { CLIENT_CAPABILITIES_META_KEY: {
            "elicitation": { "url": {} }
        }}});
        assert!(client_supports_url_elicitation(&declared));

        // An empty elicitation object means form mode only — never a URL.
        let form_only = json!({ "_meta": { CLIENT_CAPABILITIES_META_KEY: {
            "elicitation": {}
        }}});
        assert!(!client_supports_url_elicitation(&form_only));
        assert!(!client_supports_url_elicitation(&json!({ "_meta": {} })));
        assert!(!client_supports_url_elicitation(&json!({})));
    }

    #[test]
    fn result_carries_the_elicitation_under_its_key() {
        let intent = intent();
        let result = url_elicitation_result(&intent, "https://app.example.com/x", "state");
        assert_eq!(result["resultType"], "input_required");
        assert_eq!(result["requestState"], "state");
        let request = &result["inputRequests"]["secret"];
        assert_eq!(request["method"], "elicitation/create");
        assert_eq!(request["params"]["mode"], "url");
        assert_eq!(request["params"]["url"], "https://app.example.com/x");
        assert!(
            request["params"]["message"]
                .as_str()
                .unwrap()
                .contains("OPENAI_API_KEY")
        );
    }

    #[test]
    fn builds_an_absolute_page_url_with_the_token() {
        let url = elicitation_url("https://app.example.com/api/", &intent(), "abc.def");
        assert_eq!(
            url,
            "https://app.example.com/api/mcp/elicitations/secret?token=abc.def"
        );
    }

    #[test]
    fn reads_the_clients_answer() {
        let accept = json!({ "inputResponses": { "secret": { "action": "accept" } } });
        assert_eq!(accepted(&accept, "secret"), Some(true));
        let decline = json!({ "inputResponses": { "secret": { "action": "decline" } } });
        assert_eq!(accepted(&decline, "secret"), Some(false));
        assert_eq!(accepted(&json!({}), "secret"), None);
    }
}
