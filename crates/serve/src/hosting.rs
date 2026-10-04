//! Hosting targets: run a serve app inside another platform's contract.
//!
//! [`start`](crate::start) serves the `/v1` wire API on its own. A hosting
//! target is a crate of its own (for example `everruns-serve-agentcore` for
//! Amazon Bedrock AgentCore Runtime) that boots the same host through
//! [`Server`], adds the platform's routes around [`Server::router`], and
//! hands every other command back to [`start`](crate::start).
//!
//! Decisions:
//! - Composition, not a trait: a target owns its binary entry and its router,
//!   and serve exposes only what a target needs (the `/v1` router, a busy
//!   signal for health checks, the AG-UI run, the schedules). A target needs
//!   no access to serve's internals, so new targets need no serve change.
//! - [`Server::new`] boots exactly like `dev` and `start`: `start` refuses
//!   missing secrets, and every agent is resolved once so a bad model or tool
//!   schema fails at boot rather than on the first request.
//! - A target that runs each session in its own microVM supplies the
//!   `[sandbox] kind = "microvm"` adapter ([`ServerBuilder::microvm`]). Apps
//!   do not change: the same `serve.toml` gets bashkit under `dev` and the
//!   real machine under the target.
//! - Experimental, like the rest of serve.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, bail};
use axum::Router;

use crate::app::{App, Mode};
use crate::host::Host;

/// What a hosting target supplies for `[sandbox] kind = "microvm"`: it adds
/// the shell and filesystem to each agent as it is built. Without one,
/// `microvm` falls back to bashkit.
pub type MicroVm = Arc<dyn Fn(everruns::AgentBuilder) -> everruns::AgentBuilder + Send + Sync>;

/// Builder for [`Server`].
#[must_use]
pub struct ServerBuilder {
    app: App,
    mode: Mode,
    data_dir: Option<PathBuf>,
    microvm: Option<MicroVm>,
}

impl ServerBuilder {
    /// Persist under `dir`. Without it everything stays in memory.
    pub fn data_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.data_dir = Some(dir.into());
        self
    }

    /// The `[sandbox] kind = "microvm"` adapter. See [`MicroVm`].
    pub fn microvm(
        mut self,
        f: impl Fn(everruns::AgentBuilder) -> everruns::AgentBuilder + Send + Sync + 'static,
    ) -> Self {
        self.microvm = Some(Arc::new(f));
        self
    }

    /// Boot. Fails when the mode is [`Mode::Start`] and a declared secret is
    /// unset, or when an agent does not resolve.
    pub fn build(self) -> crate::Result<Server> {
        let Self {
            app,
            mode,
            data_dir,
            microvm,
        } = self;
        if !app.errors().is_empty() {
            bail!(
                "{} problem(s) found during discovery: {}",
                app.errors().len(),
                app.errors().join("; ")
            );
        }
        let missing = missing_secrets(&app);
        if mode == Mode::Start && !missing.is_empty() {
            bail!("missing secrets: {}", missing.join(", "));
        }
        let host = Host::new(app.clone(), mode, data_dir)?;
        if let Some(microvm) = microvm {
            host.set_microvm(microvm);
        }
        // Resolve every agent once so a bad model or tool schema fails at boot.
        for agent in &app.inner.agents {
            host.build_agent(agent, None, false)?;
        }
        Ok(Server { host })
    }
}

/// A booted serve host, for a hosting target to wrap.
#[derive(Clone)]
pub struct Server {
    pub(crate) host: Arc<Host>,
}

impl Server {
    /// Start building a server for `app` in `mode`.
    pub fn builder(app: App, mode: Mode) -> ServerBuilder {
        ServerBuilder {
            app,
            mode,
            data_dir: None,
            microvm: None,
        }
    }

    /// Boot `app` in `mode`, persisting under `data_dir` (`None` keeps
    /// everything in memory). Shorthand for [`Server::builder`].
    pub fn new(app: App, mode: Mode, data_dir: Option<PathBuf>) -> crate::Result<Self> {
        let builder = Self::builder(app, mode);
        match data_dir {
            Some(dir) => builder.data_dir(dir),
            None => builder,
        }
        .build()
    }

    /// The app this server runs.
    pub fn app(&self) -> &App {
        &self.host.app
    }

    /// The mode it was booted in.
    pub fn mode(&self) -> Mode {
        self.host.mode
    }

    /// The build sessions are pinned to.
    pub fn build_id(&self) -> &str {
        &self.host.build_id
    }

    /// serve's `/v1` wire API (plus `/health`), to merge into a target's
    /// router.
    pub fn router(&self) -> Router {
        crate::server::router(self.host.clone())
    }

    /// Whether a turn is running right now. A turn parked on an approval or
    /// a question is not busy: nothing runs until a person answers.
    pub fn busy(&self) -> bool {
        self.host.busy()
    }

    /// The agent a request lands on when it names none: the `default` agent,
    /// or the only top-level one.
    pub fn default_agent(&self) -> Option<String> {
        self.host
            .app
            .default_agent()
            .map(|agent| agent.name.to_string())
    }

    /// Run the app's `#[schedule]`s in-process, as `dev` and `start` do.
    pub fn spawn_schedules(&self) {
        crate::scheduler::spawn(&self.host);
    }

    /// One AG-UI run of `agent` from a raw `RunAgentInput` JSON body: the
    /// AG-UI 1.0 event stream, or the problem `POST /v1/channels/{agent}/ag-ui`
    /// would answer. Requires the `ag-ui` feature.
    #[cfg(feature = "ag-ui")]
    pub async fn ag_ui(&self, agent: &str, body: &[u8]) -> axum::response::Response {
        crate::ag_ui::respond(&self.host, agent, body).await
    }
}

/// Declared secrets that are unset or empty in this process.
pub(crate) fn missing_secrets(app: &App) -> Vec<String> {
    app.manifest()
        .secrets
        .iter()
        .map(|secret| secret.name.clone())
        .filter(|name| std::env::var_os(name).is_none_or(|value| value.is_empty()))
        .collect()
}

/// Where `dev` and `start` persist: the SQLite path in `DATABASE_URL`, else
/// `SERVE_DATA_DIR`, else `.serve/`.
pub fn data_dir() -> crate::Result<PathBuf> {
    if let Ok(url) = std::env::var("DATABASE_URL") {
        return match url
            .strip_prefix("sqlite://")
            .or_else(|| url.strip_prefix("sqlite:"))
        {
            Some(path) => Ok(PathBuf::from(path)),
            None => Err(anyhow!(
                "DATABASE_URL `{url}` is not SQLite; this proof of concept stores sessions in SQLite only"
            )),
        };
    }
    Ok(std::env::var_os("SERVE_DATA_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".serve")))
}
