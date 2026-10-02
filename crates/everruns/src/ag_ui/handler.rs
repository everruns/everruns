//! A ready-made axum route for AG-UI: authorize, resolve the thread, run,
//! stream SSE. Requires the `ag-ui-axum` feature.

// Decision: the handler lives behind its own feature, not `ag-ui`, so a host
// that mounts `Session::ag_ui` on another server never compiles axum.
//
// Decision: an authorizer is required by the constructor. AG-UI input steers
// an agent, so an endpoint open to anyone has to be asked for by name
// (`Unauthenticated`) rather than be what a forgotten builder call leaves.
//
// Decision: everything that can fail does so before the stream opens, as an
// HTTP status (401, 400, 500); once `200 text/event-stream` is sent, failures
// are the run's own `RUN_ERROR`. Internal errors are logged and answered with
// a fixed message, so store and runtime details never reach the client.

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Bytes;
use axum::http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{MethodRouter, post};
use futures::StreamExt;
use serde_json::json;

use super::{AgUiError, AgUiOptions, AgUiStream, AgUiThreads, RunAgentInput};

/// How often an open AG-UI stream sends a `: keepalive` comment, so proxies
/// and clients do not close it while a turn is quiet.
pub const SSE_KEEPALIVE: Duration = Duration::from_secs(15);

/// An AG-UI run as a `text/event-stream` response: one unnamed `data:` event
/// per AG-UI event (`data: <json>\n\n`) and a `: keepalive` comment every
/// [`SSE_KEEPALIVE`]. Requires the `ag-ui-axum` feature.
///
/// ```
/// # #[tokio::main]
/// # async fn main() -> Result<(), Box<dyn std::error::Error>> {
/// use everruns::ag_ui::{Message, RunAgentInput, sse_response};
/// use everruns::{Agent, Engine, Model};
///
/// let agent = Agent::builder().instructions("Hi.").model(Model::simulated("ok")).build()?;
/// let session = Engine::new().create(agent);
/// let input = RunAgentInput {
///     messages: vec![Message::user("m1", "Hi")],
///     ..RunAgentInput::default()
/// };
/// let response = sse_response(session.ag_ui(input).await?);
/// assert_eq!(response.headers()["content-type"], "text/event-stream");
/// # Ok(())
/// # }
/// ```
pub fn sse_response(run: AgUiStream) -> Response {
    let events = run.map(|event| {
        // Serializing a wire event cannot fail; an empty object keeps the
        // frame well formed if it ever did.
        let data = serde_json::to_string(&event).unwrap_or_else(|_| "{}".to_string());
        Ok::<_, Infallible>(SseEvent::default().data(data))
    });
    Sse::new(events)
        .keep_alive(KeepAlive::new().interval(SSE_KEEPALIVE).text("keepalive"))
        .into_response()
}

/// Who an authorized request comes from, as far as threads are concerned.
///
/// A scoped caller's threads are its own: the same `threadId` from another
/// scope names another thread ([`AgUiThreads::run_in`]).
///
/// ```
/// use everruns::ag_ui::AgUiCaller;
///
/// let caller = AgUiCaller::scoped("user-42");
/// assert_eq!(caller.scope(), Some("user-42"));
/// assert_eq!(AgUiCaller::unscoped().scope(), None);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgUiCaller {
    scope: Option<String>,
}

impl AgUiCaller {
    /// A caller whose threads share one namespace with every other unscoped
    /// caller: right for a single-user host or a shared service token.
    pub fn unscoped() -> Self {
        Self { scope: None }
    }

    /// A caller whose threads are scoped to `id`, such as an authenticated
    /// user id.
    pub fn scoped(id: impl Into<String>) -> Self {
        Self {
            scope: Some(id.into()),
        }
    }

    /// The caller's thread scope, if any.
    pub fn scope(&self) -> Option<&str> {
        self.scope.as_deref()
    }
}

/// A refused request: answered `401 Unauthorized` before the body is read.
///
/// ```
/// use everruns::ag_ui::Unauthorized;
///
/// let refused = Unauthorized::new().challenge("Bearer");
/// # let _ = refused;
/// ```
#[derive(Debug, Clone, Default)]
pub struct Unauthorized {
    challenge: Option<String>,
}

impl Unauthorized {
    /// A refusal with no `WWW-Authenticate` challenge.
    pub fn new() -> Self {
        Self::default()
    }

    /// Answer with `WWW-Authenticate: <challenge>`, such as `Bearer`.
    pub fn challenge(mut self, challenge: impl Into<String>) -> Self {
        self.challenge = Some(challenge.into());
        self
    }
}

/// Decides whether a request may run, from its headers.
///
/// Implemented for [`StaticToken`], [`Unauthenticated`], and any
/// `Fn(&HeaderMap) -> Result<AgUiCaller, Unauthorized>`; implement it for an
/// authorizer that needs to await, such as token introspection.
///
/// ```
/// use everruns::ag_ui::{AgUiAuthorizer, AgUiCaller, Unauthorized};
/// use axum::http::HeaderMap;
///
/// // A closure: the user id a trusted proxy put in a header scopes threads.
/// let from_proxy = |headers: &HeaderMap| {
///     headers
///         .get("x-user-id")
///         .and_then(|value| value.to_str().ok())
///         .map(AgUiCaller::scoped)
///         .ok_or_else(Unauthorized::new)
/// };
/// fn is_authorizer(_: &impl AgUiAuthorizer) {}
/// is_authorizer(&from_proxy);
/// ```
#[async_trait]
pub trait AgUiAuthorizer: Send + Sync + 'static {
    /// The caller behind `headers`, or why it may not run.
    async fn authorize(&self, headers: &HeaderMap) -> Result<AgUiCaller, Unauthorized>;
}

#[async_trait]
impl<F> AgUiAuthorizer for F
where
    F: Fn(&HeaderMap) -> Result<AgUiCaller, Unauthorized> + Send + Sync + 'static,
{
    async fn authorize(&self, headers: &HeaderMap) -> Result<AgUiCaller, Unauthorized> {
        self(headers)
    }
}

/// Accepts every request as [`AgUiCaller::unscoped`]. For local development,
/// or behind a proxy that already authenticates; anyone who can reach the
/// route then steers the agent.
///
/// ```
/// use everruns::ag_ui::{AgUiHandler, AgUiThreads, Unauthenticated};
/// use everruns::{Agent, Engine, Model};
///
/// let agent = Agent::builder().instructions("Hi.").model(Model::simulated("ok")).build()?;
/// let handler = AgUiHandler::new(AgUiThreads::new(Engine::new(), agent), Unauthenticated);
/// # let _ = handler;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Copy, Default)]
pub struct Unauthenticated;

#[async_trait]
impl AgUiAuthorizer for Unauthenticated {
    async fn authorize(&self, _headers: &HeaderMap) -> Result<AgUiCaller, Unauthorized> {
        Ok(AgUiCaller::unscoped())
    }
}

/// Accepts requests that present one shared secret, compared in constant
/// time; callers are [`AgUiCaller::unscoped`].
///
/// ```
/// use axum::http::{HeaderMap, HeaderValue};
/// use everruns::ag_ui::{AgUiAuthorizer, StaticToken};
///
/// # #[tokio::main]
/// # async fn main() {
/// let token = StaticToken::bearer("s3cret");
/// let mut headers = HeaderMap::new();
/// headers.insert("authorization", HeaderValue::from_static("Bearer s3cret"));
/// assert!(token.authorize(&headers).await.is_ok());
/// headers.insert("authorization", HeaderValue::from_static("Bearer guess"));
/// assert!(token.authorize(&headers).await.is_err());
/// # }
/// ```
#[derive(Clone)]
pub struct StaticToken {
    header: HeaderName,
    token: Vec<u8>,
    bearer: bool,
}

impl std::fmt::Debug for StaticToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StaticToken")
            .field("header", &self.header)
            .field("bearer", &self.bearer)
            .finish_non_exhaustive()
    }
}

impl StaticToken {
    /// Require `Authorization: Bearer <token>`.
    pub fn bearer(token: impl Into<String>) -> Self {
        Self {
            header: header::AUTHORIZATION,
            token: token.into().into_bytes(),
            bearer: true,
        }
    }

    /// Require `<header>: <token>`, such as `x-api-key`.
    pub fn header(header: HeaderName, token: impl Into<String>) -> Self {
        Self {
            header,
            token: token.into().into_bytes(),
            bearer: false,
        }
    }
}

#[async_trait]
impl AgUiAuthorizer for StaticToken {
    async fn authorize(&self, headers: &HeaderMap) -> Result<AgUiCaller, Unauthorized> {
        let refused = || {
            let refusal = Unauthorized::new();
            if self.bearer {
                refusal.challenge("Bearer")
            } else {
                refusal
            }
        };
        let presented = headers
            .get(&self.header)
            .map(HeaderValue::as_bytes)
            .ok_or_else(refused)?;
        let presented = if self.bearer {
            strip_bearer(presented).ok_or_else(refused)?
        } else {
            presented
        };
        // An empty configured token would accept an empty header.
        if self.token.is_empty() || !constant_time_eq(&self.token, presented) {
            return Err(refused());
        }
        Ok(AgUiCaller::unscoped())
    }
}

/// The credential after a case-insensitive `Bearer ` prefix.
fn strip_bearer(value: &[u8]) -> Option<&[u8]> {
    let (scheme, rest) = value.split_at_checked(7)?;
    scheme
        .eq_ignore_ascii_case(b"bearer ")
        .then(|| rest.trim_ascii())
}

/// Whether `expected` and `presented` are equal, in time that depends on
/// neither's contents nor on where they differ; only the presented length,
/// which the caller already knows, shapes the loop.
// THREAT[TM-AUTH-030]: a short-circuiting compare would leak the token byte by
// byte through response timing.
fn constant_time_eq(expected: &[u8], presented: &[u8]) -> bool {
    let mut diff = u8::from(expected.len() != presented.len());
    for (index, byte) in presented.iter().enumerate() {
        let other = expected
            .get(index % expected.len().max(1))
            .copied()
            .unwrap_or(0);
        diff |= byte ^ other;
    }
    std::hint::black_box(diff) == 0
}

/// A ready-made AG-UI endpoint for axum: authorizes each request, resolves
/// its `threadId` through [`AgUiThreads`] (scoped to the caller), runs it
/// with the host's [`AgUiOptions`], and streams the events as SSE
/// ([`sse_response`]). Requires the `ag-ui-axum` feature.
///
/// Refusals happen before the stream opens: `401` when the authorizer
/// refuses (before the body is read), `400` for a body that is not a
/// `RunAgentInput` or input the run cannot use, `500` for a store or session
/// failure (logged; the response carries a fixed message). The request body
/// is capped by axum's default 2 MB limit; add a `DefaultBodyLimit` layer to
/// change it.
///
/// ```
/// use axum::Router;
/// use everruns::ag_ui::{AgUiHandler, AgUiOptions, AgUiThreads, InterruptGate, StaticToken};
/// use everruns::{Agent, Engine, Model};
///
/// let gate = InterruptGate::new();
/// let agent = Agent::builder()
///     .instructions("Be brief.")
///     .model(Model::simulated("Hello."))
///     .ask_user(gate.clone())
///     .approver(gate.clone())
///     .build()?;
/// let handler = AgUiHandler::new(
///     AgUiThreads::new(Engine::new(), agent),
///     StaticToken::bearer("s3cret"),
/// )
/// .options(AgUiOptions::new().gate(gate));
/// let app: Router = Router::new().route("/ag-ui", handler.route());
/// # let _ = app;
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Clone)]
pub struct AgUiHandler {
    threads: AgUiThreads,
    authorizer: Arc<dyn AgUiAuthorizer>,
    options: AgUiOptions,
}

impl std::fmt::Debug for AgUiHandler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgUiHandler")
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl AgUiHandler {
    /// An endpoint over `threads`, admitting what `authorizer` accepts, with
    /// the default [`AgUiOptions`].
    pub fn new(threads: AgUiThreads, authorizer: impl AgUiAuthorizer) -> Self {
        Self {
            threads,
            authorizer: Arc::new(authorizer),
            options: AgUiOptions::new(),
        }
    }

    /// The options every run uses: projection policy, interrupt gate,
    /// trusted instructions.
    pub fn options(mut self, options: AgUiOptions) -> Self {
        self.options = options;
        self
    }

    /// A `POST` route answering AG-UI requests, for any router state.
    pub fn route<S>(self) -> MethodRouter<S>
    where
        S: Clone + Send + Sync + 'static,
    {
        post(
            move |headers: HeaderMap, body: Bytes| async move { self.handle(&headers, &body).await },
        )
    }

    /// Answer one request: the route's body, for hosts that extract the
    /// request themselves.
    pub async fn handle(&self, headers: &HeaderMap, body: &[u8]) -> Response {
        let caller = match self.authorizer.authorize(headers).await {
            Ok(caller) => caller,
            Err(refused) => return unauthorized(refused),
        };
        let input: RunAgentInput = match serde_json::from_slice(body) {
            Ok(input) => input,
            Err(error) => {
                return problem(
                    StatusCode::BAD_REQUEST,
                    &format!("invalid RunAgentInput: {error}"),
                );
            }
        };
        let options = self.options.clone();
        let run = match caller.scope() {
            Some(scope) => self.threads.run_in(scope, input, options).await,
            None => self.threads.run(input, options).await,
        };
        match run {
            Ok(run) => sse_response(run),
            Err(AgUiError::InvalidInput(why)) => problem(StatusCode::BAD_REQUEST, &why),
            Err(error) => {
                tracing::error!(error = %error, "AG-UI run failed to start");
                problem(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "the run could not be started",
                )
            }
        }
    }
}

fn unauthorized(refused: Unauthorized) -> Response {
    let mut response = problem(StatusCode::UNAUTHORIZED, "unauthorized");
    if let Some(challenge) = refused
        .challenge
        .and_then(|challenge| HeaderValue::from_str(&challenge).ok())
    {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, challenge);
    }
    response
}

/// An RFC 9457 problem response.
fn problem(status: StatusCode, detail: &str) -> Response {
    let body = json!({
        "type": "about:blank",
        "title": status.canonical_reason().unwrap_or_default(),
        "status": status.as_u16(),
        "detail": detail,
    });
    (
        status,
        [(header::CONTENT_TYPE, "application/problem+json")],
        body.to_string(),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_compares_whole_values() {
        assert!(constant_time_eq(b"token", b"token"));
        assert!(!constant_time_eq(b"token", b"tokem"));
        assert!(!constant_time_eq(b"token", b"tok"));
        assert!(!constant_time_eq(b"token", b"tokentoken"));
        assert!(!constant_time_eq(b"token", b""));
        assert!(!constant_time_eq(b"", b"x"));
    }

    #[test]
    fn bearer_prefix_is_case_insensitive() {
        assert_eq!(strip_bearer(b"Bearer abc"), Some(&b"abc"[..]));
        assert_eq!(strip_bearer(b"bearer  abc "), Some(&b"abc"[..]));
        assert_eq!(strip_bearer(b"Basic abc"), None);
        assert_eq!(strip_bearer(b"Bear"), None);
    }
}
