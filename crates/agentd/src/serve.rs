//! `agentd serve`: a whole install in one process (agentd design,
//! "Deployment shapes"): a one-node shard, an executor that runs its turns,
//! the gateway with the push API and the stream, on one public port.
//!
//! ```text
//! agentd serve --dev                          # localhost, no auth
//! agentd serve --data /var/lib/agentd --listen 0.0.0.0:7700 \
//!              --api-keys-file /etc/agentd/keys --store s3://bucket/agentd
//! ```
//!
//! Decisions:
//! - The node and the executor talk over a second listener on loopback
//!   (its address kept in the data directory, so the shard's membership
//!   stays valid across restarts). The public port serves only the API:
//!   the shard's own routes (proposals, Raft) are never reachable from
//!   outside, whatever address the API listens on.
//! - The install is its own shards' control plane (`install_stream.rs`):
//!   the node is linked to it over loopback with a token made at start, so
//!   spend is metered per group and the feed becomes the stream with no
//!   second mechanism.
//! - The data plane key, which wraps every group's data key, comes from
//!   `AGENTD_DP_KEY` or is made once in `<data>/dp.key` (mode 0600). Losing
//!   it loses every secret pushed to the install, nothing else.
//! - The object store defaults to `<data>/store`: on one disk, the stream,
//!   history and restore points go with the disk (documented: dev and small
//!   teams). Give `--store` an object store for anything else.
//! - Without `--dev` an install refuses to start with no credential: no
//!   anonymous mode outside loopback. `--dev` refuses any address but
//!   loopback.
//! - Environment model keys are off unless `--env-secrets`: then each
//!   provider key found in the environment is pushed as an install secret
//!   bound to its provider, read by the executor at the turn.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use everruns_contracts::dataplane_seal as seal;

use crate::cluster::TopologySource;
use crate::host::Host;
use crate::install::{Bind, INSTALL, Install, SecretBody};
use crate::install_auth::Auth;
use crate::node::{ExecutorMode, NodeConfig};
use crate::node_control::{ControlPlane, DpKey};

pub struct ServeConfig {
    pub data: PathBuf,
    pub listen: SocketAddr,
    /// `None`: `file://<data>/store`.
    pub store: Option<String>,
    pub group_label: Option<String>,
    pub dev: bool,
    pub env_secrets: bool,
    pub auth: Auth,
    /// Turns run at once.
    pub slots: usize,
    pub memory_budget_mb: u64,
    /// TLS for the public port (`install_tls.rs`).
    pub tls: Option<crate::install_tls::TlsFiles>,
}

/// The provider keys `--env-secrets` looks for: (variable, provider).
pub const ENV_PROVIDERS: &[(&str, &str)] = &[
    ("OPENAI_API_KEY", "openai"),
    ("ANTHROPIC_API_KEY", "anthropic"),
    ("OPENROUTER_API_KEY", "openrouter"),
    ("GEMINI_API_KEY", "gemini"),
    ("MISTRAL_API_KEY", "mistral"),
];

/// A running install (tests read its address).
pub struct Served {
    pub addr: SocketAddr,
    pub install: Arc<Install>,
    tasks: Vec<tokio::task::JoinHandle<anyhow::Result<()>>>,
}

impl Served {
    /// Run until a task stops; its error, if any.
    pub async fn wait(mut self) -> anyhow::Result<()> {
        let tasks = std::mem::take(&mut self.tasks);
        let (done, _, rest) = futures::future::select_all(tasks).await;
        for task in rest {
            task.abort();
        }
        done?
    }

    pub fn stop(&self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl Drop for Served {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The data plane key from `AGENTD_DP_KEY`, else `<data>/dp.key`, made once.
pub fn dp_key(data: &Path) -> anyhow::Result<seal::Key> {
    if let Some(key) = crate::executor::data_plane_key()? {
        return Ok(key);
    }
    let path = data.join("dp.key");
    if let Ok(text) = std::fs::read_to_string(&path) {
        return seal::key_from_base64(text.trim())
            .map_err(|e| anyhow::anyhow!("{}: {e:?}", path.display()));
    }
    let key = seal::random_key();
    use base64::Engine as _;
    let text = base64::engine::general_purpose::STANDARD.encode(key);
    write_private(&path, text.as_bytes())?;
    Ok(key)
}

fn write_private(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(bytes)?;
    Ok(())
}

/// The loopback address the node keeps across restarts.
async fn internal_listener(data: &Path) -> anyhow::Result<tokio::net::TcpListener> {
    let path = data.join("internal.addr");
    if let Ok(addr) = std::fs::read_to_string(&path) {
        return Ok(tokio::net::TcpListener::bind(addr.trim()).await?);
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    std::fs::write(&path, listener.local_addr()?.to_string())?;
    Ok(listener)
}

/// Start an install; it runs until [`Served`] is dropped or stopped.
pub async fn start(cfg: ServeConfig, host: Arc<dyn Host>) -> anyhow::Result<Served> {
    if cfg.dev {
        anyhow::ensure!(
            cfg.listen.ip().is_loopback(),
            "--dev serves without auth, so only on a loopback address (not {})",
            cfg.listen
        );
    } else {
        anyhow::ensure!(
            cfg.auth.has_credentials() || cfg.tls.as_ref().is_some_and(|t| t.client_ca.is_some()),
            "an install needs credentials: --api-keys-file, AGENTD_API_KEYS, AGENTD_JWKS_URL or --client-ca (or --dev on localhost)"
        );
    }
    std::fs::create_dir_all(&cfg.data)?;
    let dp_key = dp_key(&cfg.data)?;
    let store_url = match &cfg.store {
        Some(url) => url.clone(),
        None => format!(
            "file://{}",
            std::fs::canonicalize(&cfg.data)?.join("store").display()
        ),
    };
    let store = crate::store::Store::open(&store_url)?;
    crate::secret_sources::allow_env(cfg.env_secrets);

    let internal = internal_listener(&cfg.data).await?;
    let internal_addr = internal.local_addr()?;
    let topology_path = cfg.data.join("topology.json");
    let topology = crate::cluster::Topology {
        directory_shard: 0,
        shards: [(0, [(1, internal_addr.to_string())].into())].into(),
    };
    std::fs::write(&topology_path, serde_json::to_vec_pretty(&topology)?)?;
    let source = TopologySource {
        path: topology_path,
    };

    use base64::Engine as _;
    let link_token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(seal::random_key());
    let auth = cfg.auth.clone();
    let install = Install::new(
        INSTALL,
        cfg.group_label.clone(),
        dp_key,
        Some(store.clone()),
        auth.clone(),
    );
    let node_cfg = NodeConfig {
        shard: 0,
        node_id: 1,
        members: topology.shards[&0].clone(),
        data_dir: cfg.data.join("node"),
        store: store.clone(),
        memory_budget: cfg.memory_budget_mb << 20,
        memory_limit: 0,
        raft_port_offset: 0,
        unload_idle: Some(Duration::from_secs(600)),
        claim_ttl: Duration::from_secs(5),
        max_turns_per_node: cfg.slots,
        pressure_turns: cfg.slots,
        snapshot_every: 20_000,
        keep_log: 256,
        executors: ExecutorMode::Pull {
            max_running: cfg.slots,
        },
        control_plane: Some(ControlPlane {
            url: format!("http://{internal_addr}"),
            data_plane: "agentd".into(),
            want_micros: 1_000_000,
            per_call_micros: 10_000,
            token: Some(link_token.clone()),
            jwks_url: None,
            dp_key: Some(DpKey(dp_key)),
        }),
    };
    let mut tasks = Vec::new();
    let node_host = host.clone();
    let node_source = source.clone();
    let token: crate::node_control::LinkToken = Some(Arc::from(link_token.as_str()));
    tasks.push(tokio::spawn(async move {
        crate::cli::serve_node_with(
            node_cfg,
            node_host,
            node_source,
            internal,
            false,
            move |gateway| {
                crate::install_stream::control_plane(gateway).layer(
                    axum::middleware::from_fn_with_state(
                        token,
                        crate::node_control::require_link_token,
                    ),
                )
            },
        )
        .await
    }));

    let executor = crate::executor::Executor::start(
        crate::executor::ExecutorConfig {
            id: crate::remote::EXECUTOR_IDS,
            topology: source.clone(),
            store: store.clone().with_cache(64 << 20),
            slots: cfg.slots,
            mirror_sessions: 1024,
            pull_wait: Duration::from_secs(1),
            dream_sweep: Duration::from_secs(60),
            memory_limit: 0,
        },
        host,
    )?;
    executor.set_dp_key(dp_key);
    executor.spawn();

    let gateway = crate::gateway::Gateway::new(source);
    gateway.set_link_token(Some(link_token));
    gateway.set_dp_key(Some(dp_key));
    gateway.set_install(install.clone());
    let app = crate::gateway::router(gateway.clone())
        .route("/health", axum::routing::get(|| async { "ok" }))
        .layer(axum::middleware::from_fn_with_state(
            auth,
            crate::install_auth::check,
        ));
    let public = tokio::net::TcpListener::bind(cfg.listen).await?;
    let addr = public.local_addr()?;
    tracing::info!(%addr, dev = cfg.dev, "agentd serving");
    let tls = cfg.tls.clone();
    tasks.push(tokio::spawn(async move {
        match tls {
            Some(files) => crate::install_tls::serve(public, app, &files).await,
            None => Ok(axum::serve(public, app.into_make_service()).await?),
        }
    }));
    tasks.push(tokio::spawn(crate::install_stream::trim_loop_task(
        gateway.clone(),
        store,
    )));
    tasks.push(tokio::spawn(bootstrap(
        gateway,
        install.clone(),
        cfg.env_secrets,
    )));
    Ok(Served {
        addr,
        install,
        tasks,
    })
}

/// Enable the install's own group once its shard elects, and push the
/// environment's provider keys when allowed. Ends when done.
async fn bootstrap(
    gateway: Arc<crate::gateway::Gateway>,
    install: Arc<Install>,
    env_secrets: bool,
) -> anyhow::Result<()> {
    let id = install.id.clone();
    let mut tries = 0u32;
    while let Err((_, error)) = install.ensure_group(&gateway, &id).await {
        tries += 1;
        if tries.is_multiple_of(20) {
            tracing::warn!(%error, "the install's group is not enabled yet");
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    if env_secrets {
        for (var, provider) in ENV_PROVIDERS {
            if std::env::var(var).is_ok_and(|v| !v.is_empty()) {
                let body = SecretBody {
                    env: Some((*var).to_owned()),
                    bind: Some(Bind {
                        model: Some((*provider).to_owned()),
                        ..Bind::default()
                    }),
                    ..SecretBody::default()
                };
                let name = format!("env-{provider}");
                if let Err((_, error)) = install.put_secret(&gateway, &id, &name, body).await {
                    tracing::warn!(%var, %error, "environment key not pushed");
                }
            }
        }
    }
    // Stay alive with the others: a finished task would end the install.
    std::future::pending::<()>().await;
    Ok(())
}
