//! Saved logins: Browserless authenticated profiles, read and written by our
//! own guarded browser.
//!
//! Decision: a login lives as a profile in the Browserless account of the
//! connection the session uses. With a personal connection, the person owns
//! it. They can also create one in the Browserless dashboard ("sign in via
//! remote browser"), which handles MFA and CAPTCHAs without the agent.
//!
//! Decision: we never pass `?profile=` to Browserless. A profile loads into the
//! browser's default context, and our persistent browser runs in a guarded
//! context of its own (egress policy, `cdp.rs`), which stays logged out
//! (measured 2026-11-21: `Browserless.saveProfile` from a guarded context
//! saved 0 cookies, and a `?profile=` browser's new context had none). So we
//! move the state ourselves: download the profile's JSON and seed the guarded
//! context (cookies into its jar, localStorage on the first visit to each
//! origin), and save by capturing the guarded context's cookies plus the
//! current page's localStorage and uploading them.

use serde_json::{Value, json};

use super::cdp::CdpSession;

/// Browserless profile names: unique per token, at most 255 characters, no
/// whitespace, `/`, `?` or `#`.
pub fn validate_login_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 255 {
        return Err("Login name must be 1 to 255 characters.".to_string());
    }
    if name
        .chars()
        .any(|c| c.is_whitespace() || matches!(c, '/' | '?' | '#' | '%' | '&'))
    {
        return Err(
            "Login name cannot contain spaces or any of / ? # % &. Use something like `github-work`."
                .to_string(),
        );
    }
    Ok(())
}

/// Browserless profile REST endpoints, authenticated with the connection's
/// token (`?token=`).
pub struct ProfileClient {
    http: reqwest::Client,
    api_token: String,
    api_base: String,
}

impl ProfileClient {
    pub fn new(api_token: String) -> Self {
        Self::with_base_url(api_token, super::browserless_api_base())
    }

    pub fn with_base_url(api_token: String, api_base: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_token,
            api_base,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}?token={}", self.api_base, path, self.api_token)
    }

    /// Saved logins on this account: name, size and last change, never state.
    pub async fn list(&self) -> Result<Vec<Value>, String> {
        let body = self.send(self.http.get(self.url("/profiles"))).await?;
        let items = body
            .as_array()
            .cloned()
            .or_else(|| body.get("profiles").and_then(Value::as_array).cloned())
            .or_else(|| body.get("data").and_then(Value::as_array).cloned())
            .unwrap_or_default();
        Ok(items
            .iter()
            .map(|item| {
                json!({
                    "name": item.get("name"),
                    "cookie_count": item.get("cookieCount"),
                    "site_count": item.get("originCount"),
                    "updated_at": item.get("updatedAt"),
                    "last_used_at": item.get("lastUsedAt"),
                })
            })
            .collect())
    }

    /// The profile's state (`{cookies, origins}`), or `None` when no profile
    /// has that name.
    pub async fn download(&self, name: &str) -> Result<Option<Value>, String> {
        let response = self
            .http
            .get(self.url(&format!("/profile/{name}/download")))
            .send()
            .await
            .map_err(|_| "Failed to reach Browserless".to_string())?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Self::read(response).await.map(Some)
    }

    /// Replace the profile's state, creating the profile when it is new.
    pub async fn save(&self, name: &str, state: &Value) -> Result<Value, String> {
        let body = json!({ "name": name, "state": state });
        let refreshed = self
            .http
            .post(self.url("/profile/refresh"))
            .json(&body)
            .send()
            .await
            .map_err(|_| "Failed to reach Browserless".to_string())?;
        if refreshed.status() != reqwest::StatusCode::NOT_FOUND {
            return Self::read(refreshed).await;
        }
        self.send(self.http.post(self.url("/profile/upload")).json(&body))
            .await
    }

    async fn send(&self, request: reqwest::RequestBuilder) -> Result<Value, String> {
        let response = request
            .send()
            .await
            .map_err(|_| "Failed to reach Browserless".to_string())?;
        Self::read(response).await
    }

    async fn read(response: reqwest::Response) -> Result<Value, String> {
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|_| "Failed to read the Browserless response".to_string())?;
        // THREAT[TM-FW-001]: report the status, never the body, which can
        // carry profile state.
        if !status.is_success() {
            return Err(format!("Browserless profile API error ({status})"));
        }
        if text.is_empty() {
            return Ok(json!({}));
        }
        serde_json::from_str(&text).map_err(|_| "Invalid JSON from Browserless".to_string())
    }
}

/// Seed the guarded context with a downloaded profile: its cookies now, and
/// each origin's localStorage on the first page load of that origin.
pub async fn restore_into(
    session: &mut CdpSession,
    state: &Value,
) -> Result<RestoreSummary, String> {
    let cookies = state
        .get("cookies")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    session.set_context_cookies(&cookies).await?;
    let mut sites = 0;
    for origin in state
        .get("origins")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let (Some(name), Some(storage)) = (
            origin.get("origin").and_then(Value::as_str),
            origin.get("localStorage").and_then(Value::as_object),
        ) else {
            continue;
        };
        if storage.is_empty() {
            continue;
        }
        session
            .send_command(
                "Page.addScriptToEvaluateOnNewDocument",
                json!({ "source": local_storage_seed_script(name, &Value::Object(storage.clone())) }),
            )
            .await?;
        sites += 1;
    }
    Ok(RestoreSummary {
        cookies: cookies.len(),
        sites_with_storage: sites,
    })
}

/// Download the saved login `name` from the connection's Browserless
/// account and seed it into `session`'s guarded context.
pub async fn load_into(
    session: &mut CdpSession,
    api_token: &str,
    name: &str,
) -> Result<RestoreSummary, String> {
    validate_login_name(name)?;
    let Some(state) = ProfileClient::new(api_token.to_string())
        .download(name)
        .await?
    else {
        return Err(format!(
            "No saved login named `{name}`. Call browserless_list_logins to see the saved ones."
        ));
    };
    restore_into(session, &state).await
}

impl RestoreSummary {
    pub fn to_json(self, name: &str) -> Value {
        json!({
            "name": name,
            "cookies": self.cookies,
            "sites_with_storage": self.sites_with_storage,
        })
    }
}

/// What a restore put into the browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RestoreSummary {
    pub cookies: usize,
    pub sites_with_storage: usize,
}

/// A page script that writes `storage` into `origin`'s localStorage once per
/// browser: the first load of that origin seeds it, later loads (and anything
/// the site changes afterwards) are left alone.
fn local_storage_seed_script(origin: &str, storage: &Value) -> String {
    let origin = serde_json::to_string(origin).unwrap_or_else(|_| "\"\"".to_string());
    let storage = serde_json::to_string(storage).unwrap_or_else(|_| "{}".to_string());
    format!(
        "(() => {{ try {{ if (location.origin !== {origin}) return; \
         const flag = '__everruns_saved_login'; \
         if (localStorage.getItem(flag)) return; \
         const items = {storage}; \
         for (const [k, v] of Object.entries(items)) localStorage.setItem(k, v); \
         localStorage.setItem(flag, '1'); }} catch (_) {{}} }})();"
    )
}

/// Capture the guarded context's state as a profile: every cookie in its jar
/// and the localStorage of the page now open.
pub async fn capture_from(session: &mut CdpSession) -> Result<Value, String> {
    let cookies = session.context_cookies().await?;
    let page = session
        .send_command(
            "Runtime.evaluate",
            json!({
                "expression": "JSON.stringify({ origin: location.origin, items: (() => { \
                    const out = {}; \
                    for (let i = 0; i < localStorage.length; i++) { \
                        const k = localStorage.key(i); \
                        if (k !== '__everruns_saved_login') out[k] = localStorage.getItem(k); \
                    } \
                    return out; })() })",
                "returnByValue": true,
            }),
        )
        .await?;
    let mut origins = Vec::new();
    if let Some(text) = page.pointer("/result/value").and_then(Value::as_str)
        && let Ok(parsed) = serde_json::from_str::<Value>(text)
        && let Some(origin) = parsed.get("origin").and_then(Value::as_str)
        && (origin.starts_with("https://") || origin.starts_with("http://"))
    {
        origins.push(json!({
            "origin": origin,
            "localStorage": parsed.get("items").cloned().unwrap_or_else(|| json!({})),
        }));
    }
    Ok(json!({ "cookies": cookies, "origins": origins }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{body_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn login_names_follow_browserless_rules() {
        assert!(validate_login_name("github-work").is_ok());
        assert!(validate_login_name("").is_err());
        assert!(validate_login_name("my login").is_err());
        assert!(validate_login_name("a/b").is_err());
        assert!(validate_login_name("a?b").is_err());
        assert!(validate_login_name(&"x".repeat(256)).is_err());
    }

    #[test]
    fn storage_seed_only_touches_its_own_origin_once() {
        let script = local_storage_seed_script("https://app.example.com", &json!({"token": "t"}));
        assert!(script.contains("location.origin !== \"https://app.example.com\""));
        assert!(script.contains("__everruns_saved_login"));
        assert!(script.contains("{\"token\":\"t\"}"));
    }

    #[tokio::test]
    async fn save_refreshes_an_existing_login() {
        let server = MockServer::start().await;
        let state = json!({"cookies": [], "origins": []});
        Mock::given(method("POST"))
            .and(path("/profile/refresh"))
            .and(query_param("token", "tok"))
            .and(body_json(json!({"name": "work", "state": state})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name": "work"})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/profile/upload"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        let client = ProfileClient::with_base_url("tok".into(), server.uri());
        assert_eq!(client.save("work", &state).await.unwrap()["name"], "work");
    }

    #[tokio::test]
    async fn save_creates_a_new_login() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/profile/refresh"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/profile/upload"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"name": "new"})))
            .expect(1)
            .mount(&server)
            .await;

        let client = ProfileClient::with_base_url("tok".into(), server.uri());
        let state = json!({"cookies": [], "origins": []});
        assert_eq!(client.save("new", &state).await.unwrap()["name"], "new");
    }

    #[tokio::test]
    async fn a_missing_login_downloads_as_none() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/profile/gone/download"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let client = ProfileClient::with_base_url("tok".into(), server.uri());
        assert_eq!(client.download("gone").await.unwrap(), None);
    }

    #[tokio::test]
    async fn errors_never_echo_the_response_body() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/profile/leaky/download"))
            .respond_with(ResponseTemplate::new(500).set_body_string("cookie=secret"))
            .mount(&server)
            .await;
        let client = ProfileClient::with_base_url("tok".into(), server.uri());
        let error = client.download("leaky").await.unwrap_err();
        assert!(!error.contains("secret"));
    }

    #[tokio::test]
    async fn list_reports_names_without_state() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/profiles"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([
                {"id": "profile_1", "name": "work", "cookieCount": 3, "originCount": 1,
                 "updatedAt": "2026-11-21T00:00:00Z", "lastUsedAt": null}
            ])))
            .mount(&server)
            .await;
        let client = ProfileClient::with_base_url("tok".into(), server.uri());
        let logins = client.list().await.unwrap();
        assert_eq!(logins[0]["name"], "work");
        assert_eq!(logins[0]["cookie_count"], 3);
    }

    /// A login restored into a guarded context comes back out of it whole:
    /// what `browserless_save_login` captures is what a later
    /// `browserless_open_browser` with `login` seeded.
    #[tokio::test]
    async fn a_restored_login_is_captured_back_from_the_guarded_context() {
        use crate::browserless::browser_egress::BrowserEgress;
        use crate::browserless::test_chromium::{connect_guarded, launch_chromium};

        let Some(browser) = launch_chromium().await else {
            eprintln!("skipping: no local Chromium found (set CHROMIUM_PATH to run)");
            return;
        };
        let mut session = connect_guarded(&browser, BrowserEgress::new(None)).await;
        let state = json!({
            "cookies": [{
                "name": "sid",
                "value": "s3cret",
                "domain": "app.example.com",
                "path": "/",
                "secure": true,
                "httpOnly": true,
                "sameSite": "Lax",
                "expires": 4102444800.0,
            }],
            "origins": [{
                "origin": "https://app.example.com",
                "localStorage": {"theme": "dark"},
            }],
        });

        let summary = restore_into(&mut session, &state).await.unwrap();
        assert_eq!(
            summary,
            RestoreSummary {
                cookies: 1,
                sites_with_storage: 1,
            }
        );

        let captured = capture_from(&mut session).await.unwrap();
        let cookies = captured["cookies"].as_array().unwrap();
        assert!(
            cookies.iter().any(|c| c["name"] == "sid"
                && c["value"] == "s3cret"
                && c["domain"] == "app.example.com"),
            "captured cookies: {cookies:?}"
        );
        // about:blank has no web origin, so no localStorage is captured.
        assert_eq!(captured["origins"], json!([]));
        session.disconnect().await;
    }
}
