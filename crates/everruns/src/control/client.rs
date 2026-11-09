//! The attached side: a second process talking to a running host.

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

use super::server::{
    AttachInfo, MAX_REQUEST_BYTES, check_private_dir, check_private_file, effective_uid, write_line,
};
use super::wire::{
    ControlCommand, ControlEvent, ControlRequest, ControlResponse, ControlSent, ControlSessionInfo,
    ControlTurn,
};

/// How long the server may take to send its first line.
const FIRST_RESPONSE_TIMEOUT: Duration = Duration::from_secs(30);

/// Why a [`ControlClient`] call failed.
#[derive(Debug)]
#[non_exhaustive]
pub enum ControlClientError {
    /// The socket, token file or directory could not be used, including
    /// paths that fail the ownership and permission checks.
    Io(io::Error),
    /// The server answered with an error.
    Rejected {
        /// Stable code, see [`ControlResponse::Error`].
        code: String,
        /// Detail.
        message: String,
    },
    /// The server ended the stream: `revoked`, `shutdown` or `session_closed`.
    Closed(String),
    /// The server sent something this client does not understand.
    Protocol(String),
}

impl std::fmt::Display for ControlClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(f, "control connection failed: {error}"),
            Self::Rejected { code, message } => write!(f, "{code}: {message}"),
            Self::Closed(reason) => write!(f, "control stream closed: {reason}"),
            Self::Protocol(detail) => write!(f, "unexpected control response: {detail}"),
        }
    }
}

impl std::error::Error for ControlClientError {}

impl From<io::Error> for ControlClientError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Talks to a [`ControlServer`](super::ControlServer) from another process.
///
/// Every call opens one connection, after checking that the control
/// directory and token file belong to this user and are private, and that the
/// process listening on the socket runs as this user too. A token rejected
/// because it just rotated is re-read once.
#[derive(Clone, Debug)]
pub struct ControlClient {
    info: AttachInfo,
}

impl ControlClient {
    /// A client for the server rooted at `dir`.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            info: AttachInfo::in_dir(dir),
        }
    }

    /// The registered sessions.
    pub async fn list(&self) -> Result<Vec<ControlSessionInfo>, ControlClientError> {
        let (first, _) = self.request(ControlCommand::List).await?;
        match first {
            ControlResponse::Sessions { sessions } => Ok(sessions),
            other => Err(unexpected(other)),
        }
    }

    /// Send a message without waiting for the turn.
    pub async fn send(
        &self,
        session: &str,
        text: impl Into<String>,
    ) -> Result<ControlSent, ControlClientError> {
        let (first, _) = self.request(send(session, text, false)).await?;
        match first {
            ControlResponse::Sent(sent) => Ok(sent),
            other => Err(unexpected(other)),
        }
    }

    /// Send a message and wait for the turn that accepts it. Dropping the
    /// future disconnects but does not cancel the turn.
    pub async fn send_and_wait(
        &self,
        session: &str,
        text: impl Into<String>,
    ) -> Result<(ControlSent, ControlTurn), ControlClientError> {
        let (first, mut lines) = self.request(send(session, text, true)).await?;
        let sent = match first {
            ControlResponse::Sent(sent) => sent,
            other => return Err(unexpected(other)),
        };
        match next_response(&mut lines).await? {
            Some(ControlResponse::Turn(turn)) => Ok((sent, turn)),
            Some(other) => Err(unexpected(other)),
            None => Err(ControlClientError::Closed("disconnected".to_owned())),
        }
    }

    /// Cancel every unfinished tracked turn of the session, returning their
    /// identities.
    pub async fn cancel(&self, session: &str) -> Result<Vec<String>, ControlClientError> {
        let command = ControlCommand::Cancel {
            session: session.to_owned(),
        };
        match self.request(command).await?.0 {
            ControlResponse::Cancelled { turns } => Ok(turns),
            other => Err(unexpected(other)),
        }
    }

    /// Stream the session's output. `after` replays durable events after
    /// that sequence first; `None` starts live.
    pub async fn attach(
        &self,
        session: &str,
        after: Option<i32>,
    ) -> Result<Attachment, ControlClientError> {
        let command = ControlCommand::Attach {
            session: session.to_owned(),
            after,
        };
        let (first, lines) = self.request(command).await?;
        match first {
            ControlResponse::Attached { session } => Ok(Attachment {
                session,
                lines: Some(lines),
            }),
            other => Err(unexpected(other)),
        }
    }

    async fn request(
        &self,
        command: ControlCommand,
    ) -> Result<(ControlResponse, Connection), ControlClientError> {
        let attempt = self.request_once(command.clone()).await;
        match attempt {
            // The token may have rotated between reading the file and the
            // server checking it; one fresh read settles that race.
            Err(ControlClientError::Rejected { code, .. }) if code == "unauthorized" => {
                self.request_once(command).await
            }
            other => other,
        }
    }

    async fn request_once(
        &self,
        command: ControlCommand,
    ) -> Result<(ControlResponse, Connection), ControlClientError> {
        let uid = effective_uid();
        // THREAT[TM-AUTH-040]: never send the token or input to a socket
        // another user could have planted.
        check_private_dir(&self.info.dir, uid)?;
        check_private_file(&self.info.token_file, uid)?;
        let token = std::fs::read_to_string(&self.info.token_file)?
            .trim()
            .to_owned();
        let stream = UnixStream::connect(&self.info.socket).await?;
        if stream.peer_cred()?.uid() != uid {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "control server runs as another user",
            )
            .into());
        }
        let (read, mut write) = stream.into_split();
        let written = write_line(&mut write, &ControlRequest { token, command }).await;
        let mut lines = Connection {
            reader: BufReader::new(read),
            _write: write,
        };
        // A server that refuses up front (`busy`, a foreign peer) may close
        // before the request is written; its answer is still readable.
        // The first answer is immediate for every command (a waiting send
        // answers `sent` before it waits); later lines have no deadline, so
        // idle attach streams and long turns are fine.
        let first = tokio::time::timeout(FIRST_RESPONSE_TIMEOUT, next_response(&mut lines))
            .await
            .map_err(|_| {
                io::Error::new(io::ErrorKind::TimedOut, "control server did not answer")
            })?;
        let first = match (written, first) {
            (_, Ok(Some(ControlResponse::Error { code, message }))) => {
                return Err(ControlClientError::Rejected { code, message });
            }
            (Err(error), _) => return Err(error.into()),
            (Ok(()), first) => first?,
        };
        match first {
            Some(ControlResponse::Error { code, message }) => {
                Err(ControlClientError::Rejected { code, message })
            }
            Some(first) => Ok((first, lines)),
            None => Err(ControlClientError::Closed("disconnected".to_owned())),
        }
    }
}

/// An open connection. The write half stays open so the server does not read
/// EOF and treat the client as gone.
struct Connection {
    reader: BufReader<OwnedReadHalf>,
    _write: OwnedWriteHalf,
}

/// A live attach stream. Dropping it disconnects; the session keeps running.
pub struct Attachment {
    session: String,
    lines: Option<Connection>,
}

impl Attachment {
    /// The attached session's id.
    pub fn session(&self) -> &str {
        &self.session
    }

    /// The next event, or `None` once the stream ended normally.
    ///
    /// Returns [`ControlClientError::Closed`] when the server ended it, for
    /// example after a revocation.
    pub async fn next(&mut self) -> Result<Option<ControlEvent>, ControlClientError> {
        let Some(lines) = self.lines.as_mut() else {
            return Ok(None);
        };
        match next_response(lines).await? {
            Some(ControlResponse::Event(event)) => Ok(Some(event)),
            Some(ControlResponse::Closed { reason }) => {
                self.lines = None;
                Err(ControlClientError::Closed(reason))
            }
            Some(ControlResponse::Error { code, message }) => {
                self.lines = None;
                Err(ControlClientError::Rejected { code, message })
            }
            Some(other) => Err(unexpected(other)),
            None => {
                self.lines = None;
                Ok(None)
            }
        }
    }
}

async fn next_response(
    connection: &mut Connection,
) -> Result<Option<ControlResponse>, ControlClientError> {
    // THREAT[TM-DOS-056]: a response line is bounded like a request line, so
    // a misbehaving server cannot grow this buffer without limit.
    let mut line = Vec::new();
    let read = (&mut connection.reader)
        .take(MAX_REQUEST_BYTES + 1)
        .read_until(b'\n', &mut line)
        .await?;
    if read == 0 {
        return Ok(None);
    }
    if line.last() != Some(&b'\n') {
        return Err(ControlClientError::Protocol(
            if read as u64 > MAX_REQUEST_BYTES {
                "response line exceeds the size limit".to_owned()
            } else {
                "truncated response line".to_owned()
            },
        ));
    }
    serde_json::from_slice(&line)
        .map(Some)
        .map_err(|error| ControlClientError::Protocol(error.to_string()))
}

fn send(session: &str, text: impl Into<String>, wait: bool) -> ControlCommand {
    ControlCommand::Send {
        session: session.to_owned(),
        text: text.into(),
        wait,
    }
}

fn unexpected(response: ControlResponse) -> ControlClientError {
    match response {
        ControlResponse::Error { code, message } => ControlClientError::Rejected { code, message },
        ControlResponse::Closed { reason } => ControlClientError::Closed(reason),
        other => ControlClientError::Protocol(format!("{other:?}")),
    }
}
