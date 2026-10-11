// The personal-agent side of the Personal Agent Protocol ("Poppy", draft
// 0.1): discovery (spec 3), Sessions (spec 4) and conversations (spec 7), as
// HTTP over the egress boundary.
//
// Design Decisions:
// - Every request goes through `EgressService`, so the SSRF guard, the
//   deployment allowlist and the audit log apply to Poppy traffic like any
//   other tool traffic. Redirects are followed here, HTTPS only, at most
//   three, and only for discovery documents: a token or conversation request
//   that answers with a redirect is a failure, so a credential never follows
//   one.
// - Assertions live 60 seconds and carry a fresh `jti`; DPoP proofs bind the
//   method, the URL without query, and the token (`ath`). A `use_dpop_nonce`
//   answer is retried once with the nonce the company gave (RFC 9449 §8).
// - Failures are plain sentences for the model. They name the step and the
//   company's error code, never a token, assertion or key.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use base64::Engine as _;
use jsonwebtoken::{Algorithm, Header};
use parking_lot::Mutex;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::channels::a2a::pact_keys::ProviderKey;
use crate::kernel_imports::{EgressRequest, EgressRequestKind, EgressService};

const JWT_BEARER: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";
const CLIENT_ASSERTION_TYPE: &str = "urn:ietf:params:oauth:client-assertion-type:jwt-bearer";
const MAX_REDIRECTS: usize = 3;
/// Longest response body read, in bytes.
const MAX_BODY: usize = 1_000_000;

/// What a company's `poppy.json` and OAuth metadata say, checked.
#[derive(Debug, Clone, PartialEq)]
pub struct Company {
    /// The domain asked for, as the person named it.
    pub domain: String,
    pub name: Option<String>,
    pub issuer: String,
    pub token_endpoint: String,
    pub revocation_endpoint: Option<String>,
    /// The conversation endpoint (`agent.protocols[type=poppy].endpoint`).
    pub conversations: Option<String>,
    /// The discovery document as served.
    pub document: Value,
}

impl Company {
    /// What the model is told about the company.
    pub fn summary(&self) -> Value {
        let document = &self.document;
        json!({
            "company": self.domain,
            "name": self.name,
            "protocol_version": document.get("protocol_version"),
            "conversations": self.conversations.is_some(),
            "agent": document.get("agent").and_then(|agent| agent.get("description")),
            "apis": document.get("apis").and_then(Value::as_array).map(|apis| {
                apis.iter()
                    .map(|api| json!({ "type": api.get("type"), "description": api.get("description") }))
                    .collect::<Vec<_>>()
            }),
        })
    }
}

/// A Session Token and the Session it belongs to.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionToken {
    pub access_token: String,
    pub expires_in: i64,
    pub session_id: String,
}

/// One answer from a company.
#[derive(Debug)]
struct Reply {
    status: u16,
    headers: BTreeMap<String, String>,
    body: Value,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// The company's `error` code, or the status.
    fn error(&self) -> String {
        match self.body.get("error").and_then(Value::as_str) {
            Some(code) => code.to_string(),
            None => format!("HTTP {}", self.status),
        }
    }

    fn asks_for_nonce(&self) -> bool {
        self.header("www-authenticate")
            .is_some_and(|value| value.contains("use_dpop_nonce"))
            || self.body.get("error").and_then(Value::as_str) == Some("use_dpop_nonce")
    }
}

/// The deployment's personal agent, speaking for one person at a time.
pub struct PoppyAgent {
    egress: Arc<dyn EgressService>,
    signing: Arc<ProviderKey>,
    client_id: String,
    /// DPoP nonces by origin (RFC 9449 §8).
    nonces: Mutex<HashMap<String, String>>,
}

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn jti() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn origin(url: &url::Url) -> String {
    url.origin().ascii_serialization()
}

/// Pure: the host a person named, as `example.com`.
pub fn company_host(company: &str) -> Result<String, String> {
    let trimmed = company.trim();
    let with_scheme = if trimmed.contains("://") {
        trimmed.to_string()
    } else {
        format!("https://{trimmed}")
    };
    let url = url::Url::parse(&with_scheme).map_err(|_| format!("`{trimmed}` is not a domain"))?;
    if url.scheme() != "https" {
        return Err("Poppy companies are reached over HTTPS".into());
    }
    let host = url
        .host_str()
        .filter(|host| host.contains('.') || *host == "localhost")
        .ok_or_else(|| format!("`{trimmed}` is not a domain"))?;
    Ok(match url.port() {
        Some(port) => format!("{}:{port}", host.to_ascii_lowercase()),
        None => host.to_ascii_lowercase(),
    })
}

fn strip_www(host: &str) -> &str {
    host.strip_prefix("www.").unwrap_or(host)
}

fn https_url(value: Option<&Value>, what: &str) -> Result<String, String> {
    let raw = value
        .and_then(Value::as_str)
        .ok_or_else(|| format!("the company's {what} is missing"))?;
    let url = url::Url::parse(raw).map_err(|_| format!("the company's {what} is not a URL"))?;
    if url.scheme() != "https" {
        return Err(format!("the company's {what} is not HTTPS"));
    }
    Ok(raw.to_string())
}

impl PoppyAgent {
    pub fn new(
        egress: Arc<dyn EgressService>,
        signing: Arc<ProviderKey>,
        client_id: String,
    ) -> Self {
        Self {
            egress,
            signing,
            client_id,
            nonces: Mutex::new(HashMap::new()),
        }
    }

    async fn send(&self, request: EgressRequest) -> Result<Reply, String> {
        let response = self
            .egress
            .send(request.require_dns_pinning().timeout_ms(30_000 + 5_000))
            .await
            .map_err(|error| format!("the company could not be reached ({error})"))?;
        if response.body.len() > MAX_BODY {
            return Err("the company's answer was too large".into());
        }
        let body = if response.body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&response.body).unwrap_or(Value::Null)
        };
        Ok(Reply {
            status: response.status,
            headers: response.headers,
            body,
        })
    }

    /// GET a public document, following HTTPS redirects.
    async fn document(&self, url: &str, what: &str) -> Result<Value, String> {
        let mut url = url::Url::parse(url).map_err(|_| format!("bad {what} URL"))?;
        for _ in 0..=MAX_REDIRECTS {
            if url.scheme() != "https" {
                return Err(format!("the company's {what} is not served over HTTPS"));
            }
            let request = EgressRequest::new(
                "GET",
                url.as_str(),
                EgressRequestKind::Other("poppy".into()),
            )
            .header("accept", "application/json");
            let reply = self.send(request).await?;
            if (300..400).contains(&reply.status) {
                let location = reply
                    .header("location")
                    .ok_or_else(|| format!("the company's {what} redirect has no location"))?;
                url = url
                    .join(location)
                    .map_err(|_| format!("the company's {what} redirect is not a URL"))?;
                continue;
            }
            if reply.status != 200 || !reply.body.is_object() {
                return Err(format!(
                    "the company's {what} could not be read ({})",
                    reply.error()
                ));
            }
            return Ok(reply.body);
        }
        Err(format!("the company's {what} redirects too many times"))
    }

    /// Spec 3: read and check the company's `poppy.json` and OAuth metadata.
    pub async fn discover(&self, company: &str) -> Result<Company, String> {
        let host = company_host(company)?;
        let document = self
            .document(
                &format!("https://{host}/.well-known/poppy.json"),
                "poppy.json",
            )
            .await?;
        let organization = document.get("organization").cloned().unwrap_or_default();
        let domain = organization
            .get("domain")
            .and_then(Value::as_str)
            .ok_or("the company's poppy.json names no organization.domain")?;
        let bare_host = host.split(':').next().unwrap_or(&host);
        if !strip_www(bare_host).eq_ignore_ascii_case(strip_www(domain)) {
            return Err(format!(
                "poppy.json at {host} is for {domain}, not for {host}"
            ));
        }
        let major = document
            .get("protocol_version")
            .and_then(Value::as_str)
            .and_then(|version| version.split('.').next())
            .unwrap_or("");
        if major != "0" {
            return Err(format!(
                "the company speaks Poppy {major}, and this client speaks 0.x"
            ));
        }
        let auth = document
            .get("auth")
            .ok_or("the company does not accept personal agents (no auth in poppy.json)")?;
        let issuer = https_url(auth.get("issuer"), "OAuth issuer")?;
        let metadata = self
            .document(
                &format!(
                    "{}/.well-known/oauth-authorization-server",
                    issuer.trim_end_matches('/')
                ),
                "OAuth metadata",
            )
            .await?;
        if metadata.get("issuer").and_then(Value::as_str) != Some(issuer.as_str()) {
            return Err("the company's OAuth metadata names another issuer".into());
        }
        let covers_domain = metadata
            .get("poppy_domains")
            .and_then(Value::as_array)
            .is_some_and(|domains| {
                domains.iter().any(|value| {
                    value
                        .as_str()
                        .is_some_and(|d| d.eq_ignore_ascii_case(domain))
                })
            });
        if !covers_domain {
            return Err("the company's OAuth server does not list its domain".into());
        }
        let conversations = document
            .get("agent")
            .and_then(|agent| agent.get("protocols"))
            .and_then(Value::as_array)
            .and_then(|protocols| {
                protocols
                    .iter()
                    .find(|protocol| protocol.get("type").and_then(Value::as_str) == Some("poppy"))
            })
            .map(|protocol| https_url(protocol.get("endpoint"), "conversation endpoint"))
            .transpose()?;
        Ok(Company {
            domain: host,
            name: organization
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string),
            token_endpoint: https_url(metadata.get("token_endpoint"), "token endpoint")?,
            revocation_endpoint: metadata
                .get("revocation_endpoint")
                .map(|value| https_url(Some(value), "revocation endpoint"))
                .transpose()?,
            issuer,
            conversations,
            document,
        })
    }

    /// `private_key_jwt` client authentication (spec 4.1).
    fn client_assertion(&self, audience: &str) -> Result<String, String> {
        let issued = now();
        self.sign(
            Some("JWT"),
            &json!({
                "iss": self.client_id, "sub": self.client_id, "aud": audience,
                "iat": issued, "exp": issued + 60, "jti": jti(),
            }),
        )
    }

    fn sign(&self, typ: Option<&str>, claims: &Value) -> Result<String, String> {
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(self.signing.kid().to_string());
        header.typ = typ.map(str::to_string);
        self.signing
            .sign_with_header(&header, claims)
            .map_err(|_| "could not sign the assertion".to_string())
    }

    /// RFC 9449 proof for `method` `url`, bound to `token` when given.
    fn proof(
        &self,
        dpop: &ProviderKey,
        method: &str,
        url: &str,
        token: Option<&str>,
    ) -> Result<String, String> {
        let parsed = url::Url::parse(url).map_err(|_| "bad URL".to_string())?;
        let mut htu = parsed.clone();
        htu.set_query(None);
        htu.set_fragment(None);
        let mut claims = json!({
            "jti": jti(), "htm": method, "htu": htu.as_str(), "iat": now(),
        });
        if let Some(token) = token {
            claims["ath"] = json!(b64(&Sha256::digest(token.as_bytes())));
        }
        if let Some(nonce) = self.nonces.lock().get(&origin(&parsed)) {
            claims["nonce"] = json!(nonce);
        }
        let mut header = Header::new(Algorithm::ES256);
        header.typ = Some("dpop+jwt".into());
        header.jwk = Some(
            serde_json::from_value(dpop.public_key_jwk())
                .map_err(|_| "could not build the DPoP key".to_string())?,
        );
        dpop.sign_with_header(&header, &claims)
            .map_err(|_| "could not sign the DPoP proof".to_string())
    }

    fn remember_nonce(&self, url: &str, reply: &Reply) {
        if let (Ok(url), Some(nonce)) = (url::Url::parse(url), reply.header("dpop-nonce")) {
            self.nonces.lock().insert(origin(&url), nonce.to_string());
        }
    }

    /// Spec 4.2: a Session for `user_id`, new or (with `session_id`) renewed.
    pub async fn start_session(
        &self,
        company: &Company,
        user_id: &str,
        dpop: &ProviderKey,
        session_id: Option<&str>,
    ) -> Result<SessionToken, String> {
        let endpoint = &company.token_endpoint;
        let issued = now();
        let assertion = self.sign(
            None,
            &json!({
                "iss": self.client_id, "sub": user_id, "aud": endpoint,
                "iat": issued, "exp": issued + 60, "jti": jti(),
            }),
        )?;
        let mut reply = None;
        for _ in 0..2 {
            let mut form = vec![
                ("grant_type", JWT_BEARER.to_string()),
                ("assertion", assertion.clone()),
                ("client_id", self.client_id.clone()),
                ("client_assertion_type", CLIENT_ASSERTION_TYPE.to_string()),
                ("client_assertion", self.client_assertion(endpoint)?),
            ];
            if let Some(session_id) = session_id {
                form.push(("session_id", session_id.to_string()));
            }
            let body = serde_urlencoded::to_string(&form)
                .map_err(|_| "could not encode the token request".to_string())?;
            let request = EgressRequest::new(
                "POST",
                endpoint.as_str(),
                EgressRequestKind::Other("poppy".into()),
            )
            .header("content-type", "application/x-www-form-urlencoded")
            .header("accept", "application/json")
            .header("dpop", self.proof(dpop, "POST", endpoint, None)?)
            .body(body.into_bytes());
            let answer = self.send(request).await?;
            let retry = answer.asks_for_nonce();
            self.remember_nonce(endpoint, &answer);
            reply = Some(answer);
            if !retry {
                break;
            }
        }
        let reply = reply.expect("one attempt runs");
        if reply.status != 200 {
            return Err(format!(
                "the company did not start a Session ({})",
                reply.error()
            ));
        }
        let body = &reply.body;
        let token_type = body.get("token_type").and_then(Value::as_str);
        if token_type != Some("DPoP") {
            return Err("the company issued a token that is not DPoP-bound".into());
        }
        Ok(SessionToken {
            access_token: body
                .get("access_token")
                .and_then(Value::as_str)
                .ok_or("the company's token answer has no access_token")?
                .to_string(),
            expires_in: body.get("expires_in").and_then(Value::as_i64).unwrap_or(0),
            session_id: body
                .get("session_id")
                .and_then(Value::as_str)
                .ok_or("the company's token answer has no session_id")?
                .to_string(),
        })
    }

    /// A DPoP-authenticated JSON request (spec 4.3), retried once for a nonce.
    async fn authed(
        &self,
        method: &str,
        url: &str,
        token: &str,
        dpop: &ProviderKey,
        body: Option<&Value>,
    ) -> Result<(u16, Value), String> {
        let mut last = None;
        for _ in 0..2 {
            let mut request =
                EgressRequest::new(method, url, EgressRequestKind::Other("poppy".into()))
                    .header("authorization", format!("DPoP {token}"))
                    .header("dpop", self.proof(dpop, method, url, Some(token))?)
                    .header("accept", "application/json");
            if let Some(body) = body {
                request = request
                    .header("content-type", "application/json")
                    .body(body.to_string().into_bytes());
            }
            let reply = self.send(request).await?;
            let retry = reply.asks_for_nonce();
            self.remember_nonce(url, &reply);
            last = Some(reply);
            if !retry {
                break;
            }
        }
        let reply = last.expect("one attempt runs");
        if (300..400).contains(&reply.status) {
            return Err("the company answered with a redirect; it was not followed".into());
        }
        Ok((reply.status, reply.body))
    }

    /// Spec 7.2, 7.3: start a conversation or send into one. `Ok` carries
    /// the status and the company's answer.
    pub async fn post_message(
        &self,
        url: &str,
        token: &str,
        dpop: &ProviderKey,
        message: &Value,
    ) -> Result<(u16, Value), String> {
        self.authed(
            "POST",
            url,
            token,
            dpop,
            Some(&json!({ "message": message })),
        )
        .await
    }

    /// Spec 7.5: the conversation's events after `cursor`.
    pub async fn read_events(
        &self,
        url: &str,
        token: &str,
        dpop: &ProviderKey,
    ) -> Result<(u16, Value), String> {
        self.authed("GET", url, token, dpop, None).await
    }

    /// Spec 7.8, 7.9: `handoff` or `close`.
    pub async fn post_action(
        &self,
        url: &str,
        token: &str,
        dpop: &ProviderKey,
    ) -> Result<(u16, Value), String> {
        self.authed("POST", url, token, dpop, Some(&json!({})))
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn company_hosts_are_https_domains() {
        assert_eq!(company_host("Example.com").unwrap(), "example.com");
        assert_eq!(
            company_host("https://shop.example.com/path").unwrap(),
            "shop.example.com"
        );
        assert_eq!(
            company_host("poppy.localhost:8443").unwrap(),
            "poppy.localhost:8443"
        );
        assert!(company_host("http://example.com").is_err());
        assert!(company_host("not a domain").is_err());
        assert!(company_host("intranet").is_err());
    }

    #[test]
    fn https_urls_only() {
        assert!(https_url(Some(&json!("https://a.example/t")), "x").is_ok());
        assert!(https_url(Some(&json!("http://a.example/t")), "x").is_err());
        assert!(https_url(None, "x").is_err());
    }
}
