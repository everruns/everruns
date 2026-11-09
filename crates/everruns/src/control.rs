//! Let a second local process drive a host's running sessions.
//!
//! A terminal host often wants `myagent attach` in another terminal: list the
//! live sessions, watch one's output, type into it, cancel its turn. The
//! Engine's sessions live in the host process, so the host serves a small
//! control plane and the attached process speaks to it:
//!
//! - the host puts the sessions it wants reachable in a [`ControlRegistry`]
//!   and starts a [`ControlServer`] on a private directory;
//! - the attached process uses a [`ControlClient`] on the same directory to
//!   `list`, `send`, `cancel` and `attach`.
//!
//! Authorization is local and explicit: same OS user only (0700 directory,
//! 0600 socket, kernel peer credentials checked on both ends), plus a bearer
//! token read from a 0600 file, rotated on a schedule and revocable. See
//! [`ControlServer`] for the token lifecycle and [`ControlRegistry::cancel`]
//! for cancel semantics.
//!
//! ```no_run
//! use everruns::control::{ControlClient, ControlRegistry, ControlServer, default_dir};
//! use everruns::{Agent, Engine, Model};
//!
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! // In the host process.
//! let agent = Agent::builder().instructions("Be brief.").model(Model::simulated("hi")).build()?;
//! let session = Engine::new().create(agent);
//! let registry = ControlRegistry::new();
//! registry.register("main", &session);
//! let server = ControlServer::start(default_dir("myagent"), registry.clone())?;
//!
//! // In the attached process.
//! let client = ControlClient::new(default_dir("myagent"));
//! let (_, turn) = client.send_and_wait("main", "hello").await?;
//! println!("{}", turn.response);
//! # drop(server);
//! # Ok(())
//! # }
//! ```
//!
//! Stability: experimental — outside the [`stability`](crate::stability) promises.

mod client;
mod registry;
mod server;
mod wire;

use std::path::PathBuf;

pub use client::{Attachment, ControlClient, ControlClientError};
pub use registry::{ControlError, ControlRegistry};
pub use server::{
    AttachInfo, ControlServer, ControlServerBuilder, DEFAULT_TOKEN_TTL, MAX_REQUEST_BYTES,
};
pub use wire::{
    ControlCommand, ControlEvent, ControlEventKind, ControlRequest, ControlResponse, ControlSent,
    ControlSessionInfo, ControlTurn,
};

const SOCKET_FILE: &str = "control.sock";
const TOKEN_FILE: &str = "token";
const LOCK_FILE: &str = "lock";

/// A per-user control directory for `app`.
///
/// `$XDG_RUNTIME_DIR/<app>` when that is set (it is already private and
/// cleared at logout), otherwise `<temp dir>/<app>-<uid>`. The server creates
/// it with mode 0700 and refuses one that exists with looser permissions or
/// another owner.
pub fn default_dir(app: &str) -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").filter(|dir| !dir.is_empty()) {
        Some(runtime) => PathBuf::from(runtime).join(app),
        None => std::env::temp_dir().join(format!("{app}-{}", server::effective_uid())),
    }
}
