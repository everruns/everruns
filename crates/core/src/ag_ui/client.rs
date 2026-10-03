//! An HTTP client for AG-UI agents (feature `client`).
//!
//! [`AgUiClient::run`] POSTs a [`RunAgentInput`] to an agent's URL and
//! returns an [`EventStream`]: the agent's server-sent events, decoded and
//! checked by the [consumer pipeline](crate::ag_ui::consumer) as they arrive.
//!
//! ```no_run
//! # async fn demo() -> Result<(), everruns_core::ag_ui::client::ClientError> {
//! use everruns_core::ag_ui::client::AgUiClient;
//! use everruns_core::ag_ui::{Message, RunAgentInput};
//!
//! let client = AgUiClient::new("https://agent.example.com/ag-ui")
//!     .with_bearer_token("secret-token");
//! let input = RunAgentInput {
//!     thread_id: "thread-1".into(),
//!     run_id: "run-1".into(),
//!     messages: vec![Message::user("u1", "Summarise the release notes")],
//!     ..RunAgentInput::default()
//! }
//! .with_protocol_version();
//! let result = client.run(&input).await?.into_result().await?;
//! println!("{}", result.text());
//! # Ok(())
//! # }
//! ```

// Decision: our own SSE reader instead of `eventsource-stream`. It is a few
// dozen lines, and owning it is what lets a single frame be bounded
// (`with_max_event_bytes`): a remote agent is untrusted, and an unbounded
// line buffer is a memory exhaustion lever.

use std::collections::VecDeque;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_util::{Stream, StreamExt};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};

use crate::ag_ui::consumer::{ProtocolError, RunConsumer, RunResult};
use crate::ag_ui::{Event, RunAgentInput};

/// The default cap on one server-sent event, in bytes.
pub const DEFAULT_MAX_EVENT_BYTES: usize = 4 * 1024 * 1024;

/// What can go wrong talking to an AG-UI agent.
#[derive(Debug)]
pub enum ClientError {
    /// The request could not be sent or the response body failed.
    Http(reqwest::Error),
    /// The agent answered with a non-success status.
    Status {
        status: u16,
        /// The start of the response body, for diagnostics.
        body: String,
    },
    /// The agent did not answer with `text/event-stream`.
    ContentType(String),
    /// An extra header was not a valid HTTP header.
    InvalidHeader(String),
    /// One server-sent event exceeded the configured size cap.
    EventTooLarge { limit: usize },
    /// A server-sent event's data was not JSON.
    InvalidJson(serde_json::Error),
    /// The agent broke the AG-UI protocol.
    Protocol(ProtocolError),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Http(err) => write!(f, "AG-UI request failed: {err}"),
            Self::Status { status, body } => {
                write!(f, "AG-UI agent answered HTTP {status}: {body}")
            }
            Self::ContentType(found) => write!(
                f,
                "AG-UI agent answered with content type '{found}', expected text/event-stream"
            ),
            Self::InvalidHeader(name) => write!(f, "invalid AG-UI request header '{name}'"),
            Self::EventTooLarge { limit } => {
                write!(f, "AG-UI event exceeded the {limit}-byte limit")
            }
            Self::InvalidJson(err) => write!(f, "AG-UI event is not valid JSON: {err}"),
            Self::Protocol(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for ClientError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Http(err) => Some(err),
            Self::InvalidJson(err) => Some(err),
            Self::Protocol(err) => Some(err),
            _ => None,
        }
    }
}

impl From<ProtocolError> for ClientError {
    fn from(err: ProtocolError) -> Self {
        Self::Protocol(err)
    }
}

/// A client for one AG-UI agent endpoint.
#[derive(Clone, Debug)]
pub struct AgUiClient {
    http: reqwest::Client,
    url: String,
    bearer_token: Option<String>,
    headers: HeaderMap,
    max_event_bytes: usize,
}

impl AgUiClient {
    /// A client for the agent at `url`, with a default HTTP client.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            url: url.into(),
            bearer_token: None,
            headers: HeaderMap::new(),
            max_event_bytes: DEFAULT_MAX_EVENT_BYTES,
        }
    }

    /// Uses `http` for requests: timeouts, proxies, TLS roots, redirect
    /// policy and DNS resolution are the caller's to choose.
    pub fn with_http_client(mut self, http: reqwest::Client) -> Self {
        self.http = http;
        self
    }

    /// Sends `Authorization: Bearer <token>`.
    pub fn with_bearer_token(mut self, token: impl Into<String>) -> Self {
        self.bearer_token = Some(token.into());
        self
    }

    /// Sends an extra header on every request. Values are marked sensitive,
    /// so they stay out of debug output.
    ///
    /// ```
    /// use everruns_core::ag_ui::client::AgUiClient;
    ///
    /// let client = AgUiClient::new("https://agent.example.com")
    ///     .with_header("x-tenant", "acme")
    ///     .unwrap();
    /// assert!(AgUiClient::new("https://agent.example.com").with_header("bad header", "x").is_err());
    /// # let _ = client;
    /// ```
    pub fn with_header(mut self, name: &str, value: &str) -> Result<Self, ClientError> {
        let header = HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| ClientError::InvalidHeader(name.to_owned()))?;
        let mut value = HeaderValue::from_str(value)
            .map_err(|_| ClientError::InvalidHeader(name.to_owned()))?;
        value.set_sensitive(true);
        self.headers.insert(header, value);
        Ok(self)
    }

    /// Caps the size of one server-sent event (default
    /// [`DEFAULT_MAX_EVENT_BYTES`]).
    pub fn with_max_event_bytes(mut self, limit: usize) -> Self {
        self.max_event_bytes = limit;
        self
    }

    /// The agent URL.
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Starts a run and returns its event stream.
    pub async fn run(&self, input: &RunAgentInput) -> Result<EventStream, ClientError> {
        let mut request = self
            .http
            .post(&self.url)
            .headers(self.headers.clone())
            .header(ACCEPT, "text/event-stream")
            .json(input);
        if let Some(token) = &self.bearer_token {
            let mut value = HeaderValue::from_str(&format!("Bearer {token}"))
                .map_err(|_| ClientError::InvalidHeader(AUTHORIZATION.to_string()))?;
            value.set_sensitive(true);
            request = request.header(AUTHORIZATION, value);
        }
        let response = request.send().await.map_err(ClientError::Http)?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(ClientError::Status {
                status: status.as_u16(),
                body: body.chars().take(2048).collect(),
            });
        }
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        if !content_type
            .to_ascii_lowercase()
            .starts_with("text/event-stream")
        {
            return Err(ClientError::ContentType(content_type));
        }
        let body = response
            .bytes_stream()
            .map(|chunk| chunk.map(|bytes| bytes.to_vec()));
        Ok(EventStream {
            body: Box::pin(body),
            sse: SseReader::new(self.max_event_bytes),
            consumer: Some(RunConsumer::for_input(input)),
            finished: None,
            queue: VecDeque::new(),
            done: false,
        })
    }
}

type ByteStream = Pin<Box<dyn Stream<Item = Result<Vec<u8>, reqwest::Error>> + Send>>;

/// A run's events, decoded and verified as they arrive.
///
/// It is a [`Stream`] of the events an application should apply, with the
/// `*_CHUNK` shorthand already expanded. The first protocol violation, and a
/// body that ends inside a run, end it with an error.
/// [`into_result`](Self::into_result) drains it into a [`RunResult`].
/// Dropping it closes the connection, which is how a run is cancelled from
/// the consumer side.
pub struct EventStream {
    body: ByteStream,
    sse: SseReader,
    consumer: Option<RunConsumer>,
    finished: Option<RunResult>,
    queue: VecDeque<Event>,
    done: bool,
}

impl std::fmt::Debug for EventStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventStream")
            .field("queued", &self.queue.len())
            .field("done", &self.done)
            .finish_non_exhaustive()
    }
}

impl EventStream {
    /// The result assembled so far; complete once the stream has ended.
    pub fn result(&self) -> Option<&RunResult> {
        self.finished
            .as_ref()
            .or_else(|| self.consumer.as_ref().map(RunConsumer::result))
    }

    /// Reads the rest of the stream and returns the run's result.
    pub async fn into_result(mut self) -> Result<RunResult, ClientError> {
        while let Some(event) = self.next().await {
            event?;
        }
        self.finished.take().ok_or_else(|| {
            ClientError::Protocol(ProtocolError::new("the stream ended without a result"))
        })
    }

    fn fail(&mut self, err: ClientError) -> Poll<Option<Result<Event, ClientError>>> {
        self.done = true;
        self.consumer = None;
        self.queue.clear();
        Poll::Ready(Some(Err(err)))
    }

    fn accept(&mut self, data: &str) -> Result<(), ClientError> {
        let value: serde_json::Value =
            serde_json::from_str(data).map_err(ClientError::InvalidJson)?;
        if let Some(consumer) = self.consumer.as_mut() {
            self.queue.extend(consumer.push_value(value)?);
        }
        Ok(())
    }

    /// The body ended: apply what is left, then check the run closed.
    fn end(&mut self) -> Result<(), ClientError> {
        self.sse.flush()?;
        while let Some(data) = self.sse.ready.pop_front() {
            self.accept(&data)?;
        }
        if let Some(consumer) = self.consumer.take() {
            self.finished = Some(consumer.finish()?);
        }
        Ok(())
    }
}

impl Stream for EventStream {
    type Item = Result<Event, ClientError>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();
        loop {
            if let Some(event) = this.queue.pop_front() {
                return Poll::Ready(Some(Ok(event)));
            }
            if this.done {
                return Poll::Ready(None);
            }
            if let Some(data) = this.sse.ready.pop_front() {
                if let Err(err) = this.accept(&data) {
                    return this.fail(err);
                }
                continue;
            }
            match this.body.as_mut().poll_next(cx) {
                Poll::Pending => return Poll::Pending,
                Poll::Ready(Some(Ok(chunk))) => {
                    if let Err(err) = this.sse.feed(&chunk) {
                        return this.fail(err);
                    }
                }
                Poll::Ready(Some(Err(err))) => return this.fail(ClientError::Http(err)),
                Poll::Ready(None) => {
                    if let Err(err) = this.end() {
                        return this.fail(err);
                    }
                    this.done = true;
                }
            }
        }
    }
}

/// A minimal `text/event-stream` reader: an event's `data:` lines joined by
/// newlines, dispatched on a blank line. Other fields and comments are
/// ignored; a line ends with `\n`, `\r\n` or `\r`.
#[derive(Debug)]
struct SseReader {
    buffer: Vec<u8>,
    data: Option<String>,
    ready: VecDeque<String>,
    limit: usize,
}

impl SseReader {
    fn new(limit: usize) -> Self {
        Self {
            buffer: Vec::new(),
            data: None,
            ready: VecDeque::new(),
            limit,
        }
    }

    fn feed(&mut self, chunk: &[u8]) -> Result<(), ClientError> {
        self.buffer.extend_from_slice(chunk);
        let mut start = 0;
        while let Some(offset) = self.buffer[start..]
            .iter()
            .position(|b| *b == b'\n' || *b == b'\r')
        {
            let end = start + offset;
            let next = match (self.buffer[end], self.buffer.get(end + 1)) {
                (b'\r', Some(b'\n')) => end + 2,
                // A trailing `\r` may be the first half of `\r\n`: wait.
                (b'\r', None) => break,
                _ => end + 1,
            };
            let line = String::from_utf8_lossy(&self.buffer[start..end]).into_owned();
            start = next;
            self.line(&line)?;
        }
        self.buffer.drain(..start);
        let pending = self.buffer.len() + self.data.as_ref().map_or(0, String::len);
        if pending > self.limit {
            return Err(ClientError::EventTooLarge { limit: self.limit });
        }
        Ok(())
    }

    fn line(&mut self, line: &str) -> Result<(), ClientError> {
        if line.is_empty() {
            if let Some(data) = self.data.take() {
                self.ready.push_back(data);
            }
            return Ok(());
        }
        let (field, value) = match line.split_once(':') {
            Some((field, value)) => (field, value.strip_prefix(' ').unwrap_or(value)),
            None => (line, ""),
        };
        if field == "data" {
            match &mut self.data {
                Some(data) => {
                    data.push('\n');
                    data.push_str(value);
                }
                None => self.data = Some(value.to_owned()),
            }
            if self.data.as_ref().map_or(0, String::len) > self.limit {
                return Err(ClientError::EventTooLarge { limit: self.limit });
            }
        }
        Ok(())
    }

    /// The body ended: a last line or event without its terminator still
    /// counts.
    fn flush(&mut self) -> Result<(), ClientError> {
        let rest = std::mem::take(&mut self.buffer);
        let rest = String::from_utf8_lossy(&rest);
        let rest = rest.strip_suffix('\r').unwrap_or(&rest);
        if !rest.is_empty() {
            self.line(rest)?;
        }
        if let Some(data) = self.data.take() {
            self.ready.push_back(data);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]

    use super::*;

    fn read(chunks: &[&[u8]]) -> Vec<String> {
        let mut reader = SseReader::new(1024);
        for chunk in chunks {
            reader.feed(chunk).unwrap();
        }
        reader.flush().unwrap();
        reader.ready.drain(..).collect()
    }

    #[test]
    fn sse_reader_handles_line_endings_and_split_frames() {
        assert_eq!(read(&[b"data: a\n\ndata: b\r\n\r\n"]), ["a", "b"]);
        assert_eq!(read(&[b"data: a\r", b"\n\r\n"]), ["a"]);
        assert_eq!(read(&[b"data: a\r\rdata: b\r\r"]), ["a", "b"]);
        assert_eq!(read(&[b"da", b"ta: {\"x\"", b":1}\n\n"]), [r#"{"x":1}"#]);
        assert_eq!(
            read(&[b": comment\nevent: x\ndata: 1\ndata: 2\n\n"]),
            ["1\n2"]
        );
        assert_eq!(read(&[b"data: tail"]), ["tail"]);
    }

    #[test]
    fn sse_reader_bounds_one_event() {
        let mut reader = SseReader::new(8);
        assert!(matches!(
            reader.feed(b"data: 0123456789\n"),
            Err(ClientError::EventTooLarge { limit: 8 })
        ));
        let mut reader = SseReader::new(8);
        assert!(reader.feed(b"data: 0123456789").is_err());
    }
}
