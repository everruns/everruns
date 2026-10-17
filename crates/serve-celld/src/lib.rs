//! Native Engine hosting on celld. One Durable Object authority owns each
//! Framework session; replaceable containers run the ordinary serve Engine.
//! Canonical logs, receipts and epochs are never snapshotted or HTTP-replayed.
//!
//! OS tools use a separately provisioned workspace. Container disk is ephemeral
//! unless the operator supplies persistent storage at `SERVE_WORKSPACE_DIR`.

use anyhow::Context;
use axum::Router;
use clap::Parser;
use everruns_durable_object::{AuthorityClient, Executor, ObjectHosting};
use serve::{App, Mode, Server};
use std::{ffi::OsString, net::SocketAddr, path::PathBuf};

/// Default container listener port.
pub const PORT: u16 = 8080;

#[derive(Parser)]
#[command(about = "Run serve with per-session celld authority and native Engine execution.")]
struct Cli {
    #[arg(long,env="PORT",default_value_t=PORT)]
    port: u16,
    #[arg(long)]
    dev: bool,
}

/// Explicit authority and host configuration. Credentials stay in host memory
/// and are never part of a session binding or canonical record.
pub struct Options {
    /// Explicit sandbox adapter for OS tools. It must isolate agent commands
    /// from the Engine process and its privileged service credentials.
    pub microvm: Option<serve::MicroVm>,
    /// Independently provisioned workspace/compute services, with a retained reference.
    pub environment: Option<(
        String,
        std::sync::Arc<dyn everruns::hosting::EnvironmentFactory>,
    )>,
    /// Ordinary serve startup mode.
    pub mode: Mode,
    /// Authority transport confined to one tenant.
    pub authority: AuthorityClient,
    /// Independent credential protecting native `/execute` activations.
    pub executor_token: String,
    /// Local catalog/cache only; session durability belongs to the authority.
    pub data_dir: Option<PathBuf>,
    /// Explicit native workspace service. Its storage lifetime is independent
    /// of the session journal; omitting it refuses microVM OS tooling.
    pub workspace_dir: Option<PathBuf>,
}

impl Options {
    /// Configure a native placement without reading process environment.
    pub fn new(mode: Mode, authority: AuthorityClient, executor_token: impl Into<String>) -> Self {
        Self {
            microvm: None,
            environment: None,
            mode,
            authority,
            executor_token: executor_token.into(),
            data_dir: None,
            workspace_dir: None,
        }
    }
    /// Load the required authority URL, tenant and independent credentials.
    pub fn from_env(mode: Mode) -> serve::Result<Self> {
        let authority = AuthorityClient::new(
            &std::env::var("AUTHORITY_URL").context("AUTHORITY_URL required")?,
            &std::env::var("TENANT_ID").context("TENANT_ID required")?,
            &std::env::var("AUTHORITY_TOKEN").context("AUTHORITY_TOKEN required")?,
        )?;
        let mut options = Self::new(
            mode,
            authority,
            std::env::var("EXECUTOR_TOKEN").context("EXECUTOR_TOKEN required")?,
        );
        options.data_dir = std::env::var_os("SERVE_DATA_DIR").map(PathBuf::from);
        options.workspace_dir = std::env::var_os("SERVE_WORKSPACE_DIR").map(PathBuf::from);
        Ok(options)
    }
}

/// Run the native placement with no command or `celld`; forward other commands
/// to ordinary serve. Missing hosting configuration fails before listening.
pub async fn start(app: App) -> serve::Result {
    let mut args: Vec<OsString> = std::env::args_os().collect();
    match args.get(1).and_then(|arg| arg.to_str()) {
        None => {}
        Some("celld") => {
            args.remove(1);
        }
        Some(_) => return serve::start(app).await,
    }
    let cli = Cli::parse_from(args);
    let wire = router(
        app,
        Options::from_env(if cli.dev { Mode::Dev } else { Mode::Start })?,
    )?;
    let listener =
        tokio::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], cli.port))).await?;
    axum::serve(listener, wire)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

/// Ordinary `/v1` and authenticated `/execute` over the same Framework Engine.
/// No app-level cell or container owns canonical session state.
pub fn router(app: App, options: Options) -> serve::Result<Router> {
    // THREAT[TM-DURABLE-012]: uncontained agent commands could read the native
    // Engine's authority credentials. Workspace persistence is not a sandbox.
    if options.microvm.is_none() && app.manifest().sandbox.kind == "microvm" {
        anyhow::bail!(
            "celld native OS tools require an explicit isolated sandbox adapter; a workspace directory alone cannot isolate authority credentials"
        );
    }
    let mut builder =
        Server::builder(app, options.mode).hosting(ObjectHosting::new(options.authority.clone()));
    if let Some(dir) = options.data_dir {
        builder = builder.data_dir(dir);
    }
    if let Some(workspace) = options.workspace_dir {
        builder = builder.host_environment_factory(
            format!("celld-workspace:{}", workspace.display()),
            everruns::hosting::DirectoryEnvironmentFactory::new(workspace),
        );
    }
    if let Some((reference, factory)) = options.environment {
        builder = builder.host_environment_factory(reference, move |session| {
            let factory = factory.clone();
            async move { factory.open(session).await }
        });
    }
    if let Some(adapter) = options.microvm {
        builder = builder.microvm(move |agent| adapter(agent));
    }
    let server = builder.build()?;
    let executor = Executor::registered(
        options.authority,
        &options.executor_token,
        server.engine(),
        32,
    )?;
    Ok(server.router().merge(executor.router()))
}
