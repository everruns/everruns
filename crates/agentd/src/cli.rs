//! The shard node command line, for a node binary that brings its host
//! (model providers and tools; agents come from clients).
//!
//! ```text
//! app node    --topology cluster.json --shard 0 --node 1 --data .shard/c0n1
//! app gateway --topology cluster.json --listen 127.0.0.1:7000
//! app restore --topology cluster.json --shard 0 --store s3://... \
//!             --node 1=/var/shard/c0n1 [--at 2026-11-12T09:00:00Z | --survivor /var/shard/c0n3]
//! app restore-org --topology cluster.json --store s3://... --org <org> \
//!             --at 2026-11-12T09:00:00Z
//! app edge    --topology cluster.json --listen 127.0.0.1:7100 \
//!             --control-plane http://127.0.0.1:9000
//! ```
//! A node also serves the gateway API, so a single node is a complete local
//! runtime.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use clap::{Parser, Subcommand};

use crate::cluster::{Topology, TopologySource};
use crate::gateway::Gateway;
use crate::host::Host;
use crate::node::{Node, NodeConfig};
use crate::state::ShardId;
use crate::types::NodeId;

#[derive(Parser)]
#[command(about = "A Raft-replicated shard node for Everruns agents")]
struct Cli {
    #[command(subcommand)]
    command: Mode,
}

#[derive(Subcommand)]
enum Mode {
    /// Run one node of a shard (plus the gateway API).
    Node {
        #[arg(long, env = "AGENTD_TOPOLOGY")]
        topology: PathBuf,
        #[arg(long)]
        shard: ShardId,
        /// This node's id; unset, one more than the trailing ordinal of
        /// `HOSTNAME` (a StatefulSet pod `shard0-2` is node 3).
        #[arg(long, env = "AGENTD_NODE")]
        node: Option<NodeId>,
        #[arg(long)]
        data: PathBuf,
        /// Object store for session history and snapshot copies, shared by
        /// the whole fleet: `s3://bucket/prefix` (with `AWS_*` settings) or
        /// `file:///dir`.
        #[arg(long, env = "AGENTD_STORE")]
        store: String,
        /// Memory this node may use, in MiB: past it, idle sessions' events
        /// are unloaded (everything the process holds counts, see
        /// `event_allowance`). Leave headroom below the container limit.
        #[arg(long, env = "AGENTD_MEMORY_BUDGET_MB", default_value = "1024")]
        memory_budget_mb: u64,
        /// The memory limit this node's memory pressure is measured against,
        /// in MiB: near it the shard takes no new sessions, then no new input
        /// (see `pressure.rs`). 0: the container's limit.
        #[arg(long, env = "AGENTD_MEMORY_LIMIT_MB", default_value = "0")]
        memory_limit_mb: u64,
        /// Serve Raft on the node's port plus this, on its own threads, so
        /// client load cannot hold up replication (0: on the node's
        /// listener). Every member of a shard must use the same offset.
        #[arg(long, env = "AGENTD_RAFT_PORT_OFFSET", default_value = "0")]
        raft_port_offset: u16,
        /// Also unload sessions idle this many seconds (0: only under pressure).
        #[arg(long, env = "AGENTD_UNLOAD_IDLE_SECS", default_value = "0")]
        unload_idle_secs: u64,
        #[arg(long, default_value = "5000")]
        claim_ttl_ms: u64,
        /// Turns one node runs at once; more wait in their session's inbox.
        #[arg(long, env = "AGENTD_MAX_TURNS_PER_NODE", default_value = "256")]
        max_turns_per_node: usize,
        /// Turns the shard runs at once while it is short of memory; queued
        /// work waits until memory frees (see `pressure.rs`).
        #[arg(long, env = "AGENTD_PRESSURE_TURNS", default_value = "1024")]
        pressure_turns: usize,
        #[arg(long, default_value = "20000")]
        snapshot_every: u64,
        /// Log entries kept behind each snapshot, for followers that fell
        /// behind; further back they catch up by snapshot. The kept entries
        /// stay in memory (see `node.rs` for the measured cost).
        #[arg(long, default_value = "256")]
        keep_log: u64,
        /// Join a running shard instead of forming it: the node waits until
        /// the shard's leader adds it (`POST /v1/shards/{shard}/nodes`).
        #[arg(long)]
        join: bool,
        /// Address to serve on; required with `--join` when the topology
        /// file does not list this node.
        #[arg(long)]
        listen: Option<String>,
        /// Leave turns to executors outside the shard, which pull them with
        /// credits; at most this many run in the shard at once (0: nodes run
        /// turns themselves).
        #[arg(long, env = "AGENTD_EXECUTOR_CAP", default_value = "0")]
        executor_cap: usize,
        /// The control plane's `/dp/v1` base URL: the leader delivers the
        /// feed there and asks it for spend allowances. Every mode trusts
        /// its origin for injected capabilities and Memory (`run`).
        #[arg(long, env = "AGENTD_CONTROL_PLANE")]
        control_plane: Option<String>,
    },
    /// Run an executor: pull turns from every shard and run them.
    Executor {
        #[arg(long, env = "AGENTD_TOPOLOGY")]
        topology: PathBuf,
        /// This executor's id (unique in the fleet; at least 2^32); unset,
        /// 2^32 plus the trailing ordinal of `HOSTNAME`.
        #[arg(long, env = "AGENTD_EXECUTOR_ID")]
        id: Option<u64>,
        #[arg(long, env = "AGENTD_STORE")]
        store: String,
        /// Turns this executor runs at once.
        #[arg(long, env = "AGENTD_EXECUTOR_SLOTS", default_value = "64")]
        slots: usize,
        /// Sessions kept mirrored after their turn ended.
        #[arg(long, default_value = "4096")]
        mirror_sessions: usize,
        /// Segment and blob cache, in MiB.
        #[arg(long, default_value = "256")]
        cache_mb: usize,
        /// Address for `/health` and `/metrics`.
        #[arg(long)]
        listen: Option<SocketAddr>,
        /// Memory limit in MiB turns are admitted against; 0: the
        /// container's.
        #[arg(long, env = "AGENTD_EXECUTOR_MEMORY_LIMIT_MB", default_value = "0")]
        memory_limit_mb: u64,
    },
    /// Run the edge router: one public URL for the control plane and the
    /// data plane (`edge.rs`).
    Edge {
        #[arg(long, env = "AGENTD_TOPOLOGY")]
        topology: PathBuf,
        #[arg(long, env = "AGENTD_EDGE_LISTEN")]
        listen: SocketAddr,
        /// The control plane's base URL (its `/v1` API).
        #[arg(long, env = "AGENTD_EDGE_CONTROL_PLANE")]
        control_plane: String,
        /// The control plane's org token keys; its
        /// `/.well-known/jwks.json` when unset.
        #[arg(long, env = "AGENTD_JWKS_URL")]
        jwks_url: Option<String>,
        /// The data plane org tokens must name.
        #[arg(long, env = "AGENTD_DATA_PLANE")]
        data_plane: Option<String>,
        /// The path the API is served under (`/api` behind the local
        /// proxy); empty when it is served at the root.
        #[arg(long, env = "AGENTD_EDGE_API_PREFIX", default_value = "")]
        api_prefix: String,
    },
    /// Rebuild a shard from its restore points in the object store, as a
    /// new incarnation (`restore.rs`): after it lost quorum, or to take it
    /// back to a moment before a bad release. Stop its nodes first; the
    /// voters are the shard's members in the topology file.
    Restore {
        #[arg(long, env = "AGENTD_TOPOLOGY")]
        topology: PathBuf,
        #[arg(long)]
        shard: ShardId,
        #[arg(long, env = "AGENTD_STORE")]
        store: String,
        /// `<node>=<empty data directory>` for each node on this machine.
        #[arg(long = "node", value_parser = parse_node_dir)]
        nodes: Vec<(NodeId, PathBuf)>,
        /// The moment to restore to, Unix milliseconds or RFC 3339; the
        /// newest state without one.
        #[arg(long, value_parser = parse_at)]
        at: Option<i64>,
        /// Seed these nodes from an incarnation already rebuilt on another
        /// machine instead of rebuilding.
        #[arg(long, conflicts_with = "at")]
        incarnation: Option<u64>,
        /// The data directory of a stopped replica that survived: its
        /// snapshot and log extend the object store's (`restore::with_survivor`).
        /// Give it whenever one replica is left; it holds every write that
        /// was answered only once all replicas held it.
        #[arg(long, conflicts_with_all = ["at", "incarnation"])]
        survivor: Option<PathBuf>,
        /// Report what a restore would find, and write nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Take one org's sessions back to a moment, where they live now, on a
    /// running fleet (`restore::restore_org`). No other org changes.
    RestoreOrg {
        #[arg(long, env = "AGENTD_TOPOLOGY")]
        topology: PathBuf,
        #[arg(long, env = "AGENTD_STORE")]
        store: String,
        #[arg(long)]
        org: String,
        /// The moment to restore to, Unix milliseconds or RFC 3339.
        #[arg(long, value_parser = parse_at)]
        at: i64,
    },
    /// Where a session is and what it is doing, and how each shard keeps
    /// up (`inspect.rs`). Read only.
    Inspect {
        #[arg(long, env = "AGENTD_TOPOLOGY")]
        topology: PathBuf,
        /// Print JSON instead of text.
        #[arg(long)]
        json: bool,
        #[command(subcommand)]
        what: Inspect,
    },
    /// This container's CPU and memory and its machine's, as JSON
    /// (`container.rs`), for load runs that sample every pod. Read only.
    Usage,
    /// Check the object store operations the shard's safety rests on
    /// against the real store (`store_check.rs`), under a scratch prefix
    /// it deletes after. Prints JSON; fails when any check does.
    StoreCheck {
        #[arg(long, env = "AGENTD_STORE")]
        store: String,
    },
    /// Run a stateless gateway.
    Gateway {
        #[arg(long, env = "AGENTD_TOPOLOGY")]
        topology: PathBuf,
        #[arg(long)]
        listen: SocketAddr,
        /// Serve an install's push API and stream, and be its shards'
        /// control plane (`install.rs`; the nodes' `AGENTD_CONTROL_PLANE`).
        #[arg(long, env = "AGENTD_INSTALL")]
        install: bool,
        #[command(flatten)]
        options: InstallOptions,
    },
    /// Run a whole install in one process: a one-node shard, its executor
    /// and the API on one port (`serve.rs`).
    Serve {
        /// Where the install keeps its state.
        #[arg(long, env = "AGENTD_DATA", default_value = ".agentd")]
        data: PathBuf,
        #[arg(long, env = "AGENTD_LISTEN", default_value = "127.0.0.1:7700")]
        listen: SocketAddr,
        /// Localhost only, no auth: for development.
        #[arg(long)]
        dev: bool,
        /// Turns run at once.
        #[arg(long, env = "AGENTD_SLOTS", default_value = "64")]
        slots: usize,
        #[arg(long, env = "AGENTD_MEMORY_BUDGET_MB", default_value = "1024")]
        memory_budget_mb: u64,
        /// PEM certificate chain and key: serve TLS.
        #[arg(long, env = "AGENTD_TLS_CERT", requires = "tls_key")]
        tls_cert: Option<PathBuf>,
        #[arg(long, env = "AGENTD_TLS_KEY", requires = "tls_cert")]
        tls_key: Option<PathBuf>,
        /// PEM CA: require client certificates it signed (mTLS).
        #[arg(long, env = "AGENTD_CLIENT_CA", requires = "tls_cert")]
        client_ca: Option<PathBuf>,
        #[command(flatten)]
        options: InstallOptions,
    },
    /// Push every agent file in a folder to an install (`push.rs`).
    Push {
        /// A folder of agent definitions, one `.json` or `.yaml` per agent.
        dir: PathBuf,
        #[arg(long, env = "AGENTD_URL", default_value = "http://127.0.0.1:7700")]
        url: String,
        #[arg(long, env = "AGENTD_API_KEY")]
        api_key: Option<String>,
        /// Delete the install's agents that have no file.
        #[arg(long)]
        prune: bool,
    },
}

/// What an install is configured with, wherever it runs.
#[derive(clap::Args, Clone)]
struct InstallOptions {
    /// The object store for history, restore points and the stream
    /// (`serve`: `<data>/store` when unset).
    #[arg(long = "install-store", env = "AGENTD_STORE")]
    store: Option<String>,
    /// The label whose value groups sessions for limits, spend and keys.
    #[arg(long, env = "AGENTD_GROUP_LABEL")]
    group_label: Option<String>,
    /// Let pushed secrets come from the executor's environment, and push
    /// the provider keys found there.
    #[arg(long, env = "AGENTD_ENV_SECRETS")]
    env_secrets: bool,
    /// A file of API keys, `id:secret` per line (also `AGENTD_API_KEYS`).
    #[arg(long, env = "AGENTD_API_KEYS_FILE")]
    api_keys_file: Option<PathBuf>,
}

impl InstallOptions {
    /// An empty variable (Compose's `${X:-}`) means unset.
    fn normalized(mut self) -> Self {
        self.store = self.store.filter(|s| !s.is_empty());
        self.group_label = self.group_label.filter(|s| !s.is_empty());
        self.api_keys_file = self.api_keys_file.filter(|p| !p.as_os_str().is_empty());
        self
    }

    fn auth(&self, dev: bool) -> anyhow::Result<crate::install_auth::Auth> {
        if dev {
            return Ok(crate::install_auth::Auth::open());
        }
        let mut auth = crate::install_auth::Auth::default();
        if let Ok(keys) = std::env::var("AGENTD_API_KEYS") {
            auth = auth.with_keys(&keys)?;
        }
        if let Some(file) = &self.api_keys_file {
            auth = auth.with_keys(&std::fs::read_to_string(file)?)?;
        }
        if let Ok(url) = std::env::var("AGENTD_JWKS_URL")
            && !url.is_empty()
        {
            auth = auth.with_jwks(url);
        }
        Ok(auth)
    }
}

#[derive(Subcommand)]
enum Inspect {
    /// Every shard: leader, shipping, snapshot age, load, and each node's lag.
    Shards,
    /// One session: its directory entry, its shard, its claim and its last
    /// events.
    Session { id: String },
    /// Sessions part way through a move, with the directory's view of each.
    Moves,
}

/// The trailing `-N` of `HOSTNAME`: a StatefulSet pod's ordinal, which
/// names its node (decision: no shell in the image to compute it).
fn hostname_ordinal() -> anyhow::Result<u64> {
    let host = std::env::var("HOSTNAME")
        .map_err(|_| anyhow::anyhow!("no id given and HOSTNAME is unset"))?;
    ordinal(&host)
        .ok_or_else(|| anyhow::anyhow!("no id given and HOSTNAME {host:?} ends in no ordinal"))
}

fn ordinal(host: &str) -> Option<u64> {
    host.rsplit_once('-')?.1.parse().ok()
}

pub fn init_tracing() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,openraft=warn".into()),
        )
        .try_init();
}

/// Parse the command line and run until stopped.
pub async fn run(host: impl Host) -> anyhow::Result<()> {
    init_tracing();
    let host: Arc<dyn Host> = Arc::new(host);
    // Decision: the control plane this data plane is linked to serves its
    // injected capabilities and its sessions' Memory; it is often on an
    // internal address (a cluster service, `127.0.0.1` locally), which the
    // MCP client refuses for any other server. Every mode reads it, since
    // nodes and executors both run turns.
    if let Ok(control_plane) = std::env::var("AGENTD_CONTROL_PLANE")
        && !control_plane.is_empty()
    {
        everruns_core::mcp::trust_controller_origin(&control_plane);
    }
    // Any mode may be part of an install: executors then look up a group's
    // secrets in the install's too, and read environment-sourced ones when
    // the install allows them.
    let truthy = |name: &str| {
        std::env::var(name).is_ok_and(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
    };
    if truthy("AGENTD_INSTALL") {
        crate::install::set_current(crate::install::INSTALL);
    }
    if truthy("AGENTD_ENV_SECRETS") {
        crate::secret_sources::allow_env(true);
    }
    match Cli::parse().command {
        Mode::Node {
            topology,
            shard,
            node,
            data,
            store,
            memory_budget_mb,
            memory_limit_mb,
            raft_port_offset,
            unload_idle_secs,
            claim_ttl_ms,
            max_turns_per_node,
            pressure_turns,
            snapshot_every,
            keep_log,
            join,
            listen,
            executor_cap,
            control_plane,
        } => {
            let node = match node {
                Some(node) => node,
                None => hostname_ordinal()? + 1,
            };
            let source = TopologySource { path: topology };
            let seeded = source.read()?.shards.get(&shard).cloned();
            let members = match (&seeded, join) {
                (Some(members), false) => members.clone(),
                (None, false) => anyhow::bail!("shard {shard} not in topology (use --join)"),
                (_, true) => Default::default(),
            };
            let addr = listen
                .or_else(|| seeded.and_then(|m| m.get(&node).cloned()))
                .ok_or_else(|| anyhow::anyhow!("node {node}: no address (use --listen)"))?;
            let cfg = NodeConfig {
                shard,
                node_id: node,
                members,
                data_dir: data,
                store: crate::store::Store::open(&store)?,
                memory_budget: memory_budget_mb * 1024 * 1024,
                memory_limit: memory_limit_mb << 20,
                raft_port_offset,
                unload_idle: (unload_idle_secs > 0).then(|| Duration::from_secs(unload_idle_secs)),
                claim_ttl: Duration::from_millis(claim_ttl_ms),
                max_turns_per_node,
                pressure_turns,
                snapshot_every,
                keep_log,
                executors: if executor_cap == 0 {
                    crate::node::ExecutorMode::InNode
                } else {
                    crate::node::ExecutorMode::Pull {
                        max_running: executor_cap,
                    }
                },
                control_plane: control_plane.map(crate::node_control::ControlPlane::new),
            };
            serve_node(cfg, host, source, addr.parse()?, join).await
        }
        Mode::Executor {
            topology,
            id,
            store,
            slots,
            mirror_sessions,
            cache_mb,
            listen,
            memory_limit_mb,
        } => {
            let id = match id {
                Some(id) => id,
                None => (1 << 32) + hostname_ordinal()?,
            };
            let cfg = crate::executor::ExecutorConfig {
                id,
                topology: TopologySource { path: topology },
                store: crate::store::Store::open(&store)?.with_cache(cache_mb << 20),
                slots,
                mirror_sessions,
                pull_wait: Duration::from_secs(1),
                dream_sweep: Duration::from_secs(60),
                memory_limit: memory_limit_mb << 20,
            };
            serve_executor(cfg, host, listen).await
        }
        Mode::Edge {
            topology,
            listen,
            control_plane,
            jwks_url,
            data_plane,
            api_prefix,
        } => {
            let jwks = jwks_url.unwrap_or_else(|| {
                format!(
                    "{}{}/.well-known/jwks.json",
                    control_plane.trim_end_matches('/'),
                    api_prefix.trim_end_matches('/'),
                )
            });
            let tokens = Arc::new(crate::org_token::OrgTokens::new(Some(jwks), data_plane));
            let gateway = Gateway::new(TopologySource { path: topology });
            let edge = crate::edge::Edge::with_prefix(gateway, &control_plane, tokens, &api_prefix);
            let listener = tokio::net::TcpListener::bind(listen).await?;
            tracing::info!(%listen, control_plane, "edge listening");
            axum::serve(listener, edge.router().into_make_service()).await?;
            Ok(())
        }
        Mode::Restore {
            topology,
            shard,
            store,
            nodes,
            at,
            incarnation,
            survivor,
            dry_run,
        } => {
            let store = crate::store::Store::open(&store)?;
            let report = restore(
                &store,
                &TopologySource { path: topology },
                shard,
                &nodes.into_iter().collect(),
                RestorePoint {
                    at,
                    survivor,
                    incarnation,
                },
                dry_run,
            )
            .await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Mode::RestoreOrg {
            topology,
            store,
            org,
            at,
        } => {
            let store = crate::store::Store::open(&store)?;
            let gateway = Gateway::new(TopologySource { path: topology });
            gateway.refresh_registry().await;
            let report = crate::restore::restore_org(&store, &gateway, &org, at).await?;
            println!("{}", serde_json::to_string_pretty(&report)?);
            Ok(())
        }
        Mode::Inspect {
            topology,
            json,
            what,
        } => inspect(Topology::load(&topology)?, json, what).await,
        Mode::Usage => {
            println!("{}", serde_json::to_string(&crate::container::read())?);
            Ok(())
        }
        Mode::StoreCheck { store } => {
            let checks = crate::store_check::run(&store).await?;
            println!("{}", serde_json::to_string_pretty(&checks)?);
            let failed: Vec<&str> = checks.iter().filter(|c| !c.ok).map(|c| c.name).collect();
            if !failed.is_empty() {
                anyhow::bail!("store checks failed: {}", failed.join(", "));
            }
            Ok(())
        }
        Mode::Gateway {
            topology,
            listen,
            install,
            options,
        } => {
            let gateway = Gateway::new(TopologySource { path: topology });
            let mut app = crate::gateway::router(gateway.clone());
            if install {
                app = install_gateway(&gateway, &options)?;
            }
            let listener = tokio::net::TcpListener::bind(listen).await?;
            tracing::info!(%listen, install, "gateway listening");
            axum::serve(listener, app.into_make_service()).await?;
            Ok(())
        }
        Mode::Serve {
            data,
            listen,
            dev,
            slots,
            memory_budget_mb,
            tls_cert,
            tls_key,
            client_ca,
            options,
        } => {
            let options = options.normalized();
            let tls = match (tls_cert, tls_key) {
                (Some(cert), Some(key)) => Some(crate::install_tls::TlsFiles {
                    cert,
                    key,
                    client_ca,
                }),
                _ => None,
            };
            let cfg = crate::serve::ServeConfig {
                data,
                listen,
                store: options.store.clone(),
                group_label: options.group_label.clone(),
                dev,
                env_secrets: options.env_secrets,
                auth: options.auth(dev)?,
                slots,
                memory_budget_mb,
                tls,
            };
            let served = crate::serve::start(cfg, host).await?;
            println!("agentd serving on http://{}", served.addr);
            // SIGTERM too: in a container agentd is PID 1, which ignores a
            // signal it has no handler for, so `docker stop` would wait out
            // its grace period.
            let mut term =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
            tokio::select! {
                result = served.wait() => result,
                _ = tokio::signal::ctrl_c() => Ok(()),
                _ = term.recv() => Ok(()),
            }
        }
        Mode::Push {
            dir,
            url,
            api_key,
            prune,
        } => {
            let pushed = crate::push::push(&url, api_key.as_deref(), &dir, prune).await?;
            for p in &pushed {
                println!("{:<10} {} (revision {})", p.outcome, p.id, p.revision);
            }
            Ok(())
        }
    }
}

/// A gateway that serves an install (`gateway --install`): the push API
/// and the stream behind the install's auth, and the shards' control plane
/// behind the link's token (`AGENTD_DP_TOKEN`, which the nodes send).
fn install_gateway(
    gateway: &Arc<Gateway>,
    options: &InstallOptions,
) -> anyhow::Result<axum::Router> {
    let options = &options.clone().normalized();
    let dp_key = crate::executor::data_plane_key()?
        .ok_or_else(|| anyhow::anyhow!("an install's gateway needs AGENTD_DP_KEY"))?;
    let auth = options.auth(false)?;
    anyhow::ensure!(
        auth.has_credentials(),
        "an install needs credentials: AGENTD_API_KEYS, --api-keys-file or AGENTD_JWKS_URL"
    );
    let store = options
        .store
        .as_deref()
        .map(crate::store::Store::open)
        .transpose()?;
    crate::secret_sources::allow_env(options.env_secrets);
    let install = crate::install::Install::new(
        crate::install::INSTALL,
        options.group_label.clone(),
        dp_key,
        store.clone(),
        auth.clone(),
    );
    gateway.set_install(install);
    if let Some(store) = store {
        tokio::spawn(crate::install_stream::trim_loop(gateway.clone(), store));
    }
    let token = crate::node_control::link_token().map(Arc::from);
    Ok(crate::gateway::router(gateway.clone())
        .layer(axum::middleware::from_fn_with_state(
            auth,
            crate::install_auth::check,
        ))
        .merge(crate::install_stream::control_plane(gateway.clone()).layer(
            axum::middleware::from_fn_with_state(token, crate::node_control::require_link_token),
        )))
}

/// What the `restore` command restores from.
struct RestorePoint {
    at: Option<i64>,
    survivor: Option<PathBuf>,
    incarnation: Option<u64>,
}

/// The `restore` command: a dry run, a rebuild, or seeding nodes of a
/// rebuilt incarnation.
async fn restore(
    store: &crate::store::Store,
    topology: &TopologySource,
    shard: ShardId,
    local: &std::collections::BTreeMap<NodeId, PathBuf>,
    point: RestorePoint,
    dry_run: bool,
) -> anyhow::Result<serde_json::Value> {
    let RestorePoint {
        at,
        survivor,
        incarnation,
    } = point;
    if dry_run {
        let found = match &survivor {
            Some(dir) => crate::restore::with_survivor(store, shard, dir).await?,
            None => crate::restore::state_at(store, shard, at).await?,
        };
        return Ok(serde_json::json!({
            "shard": shard,
            "from": found.from.map(|c| serde_json::json!({ "incarnation": c.inc, "index": c.index })),
            "last_index": found.last,
            "clock_ms": found.state.clock_ms,
            "sessions": found.state.sessions.len(),
            "replayed": found.replayed,
            "from_survivor": found.from_survivor,
        }));
    }
    if local.is_empty() {
        anyhow::bail!("name the nodes on this machine with --node <id>=<dir>");
    }
    if let Some(incarnation) = incarnation {
        let mut seeded = serde_json::Map::new();
        for (node, dir) in local {
            let index = crate::restore::seed(store, shard, incarnation, *node, dir).await?;
            seeded.insert(node.to_string(), index.into());
        }
        return Ok(
            serde_json::json!({ "shard": shard, "incarnation": incarnation, "seeded": seeded }),
        );
    }
    let members = topology
        .read()?
        .shards
        .get(&shard)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("shard {shard} not in topology"))?;
    let rebuilt =
        crate::restore::rebuild(store, shard, &members, local, at, survivor.as_deref()).await?;
    Ok(serde_json::to_value(rebuilt)?)
}

fn parse_node_dir(value: &str) -> Result<(NodeId, PathBuf), String> {
    let (node, dir) = value
        .split_once('=')
        .ok_or_else(|| format!("expected <node>=<dir>, got {value}"))?;
    let node = node.parse().map_err(|e| format!("node id {node}: {e}"))?;
    Ok((node, PathBuf::from(dir)))
}

fn parse_at(value: &str) -> Result<i64, String> {
    value.parse::<i64>().or_else(|_| {
        chrono::DateTime::parse_from_rfc3339(value)
            .map(|t| t.timestamp_millis())
            .map_err(|e| format!("{value}: neither Unix milliseconds nor RFC 3339 ({e})"))
    })
}

/// Start a node and serve its API and the gateway API on `listen`.
pub async fn serve_node(
    cfg: NodeConfig,
    host: Arc<dyn Host>,
    topology: TopologySource,
    listen: SocketAddr,
    join: bool,
) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind(listen).await?;
    serve_node_on(cfg, host, topology, listener, join).await
}

/// [`serve_node`] on a listener the caller already bound, so a caller that
/// must know the address first (a test fleet) never gives the port up.
pub async fn serve_node_on(
    cfg: NodeConfig,
    host: Arc<dyn Host>,
    topology: TopologySource,
    listener: tokio::net::TcpListener,
    join: bool,
) -> anyhow::Result<()> {
    serve_node_with(cfg, host, topology, listener, join, |_| axum::Router::new()).await
}

/// [`serve_node_on`] with `extra` routes beside the node's, built from its
/// gateway (`serve.rs` adds the install's own control plane).
pub async fn serve_node_with(
    cfg: NodeConfig,
    host: Arc<dyn Host>,
    topology: TopologySource,
    listener: tokio::net::TcpListener,
    join: bool,
    extra: impl FnOnce(Arc<Gateway>) -> axum::Router,
) -> anyhow::Result<()> {
    let shard = cfg.shard;
    let id = cfg.node_id;
    let listen = listener.local_addr()?;
    let node = Node::start(cfg, host).await?;
    let gateway = Gateway::new(topology);
    if let Some(cp) = &node.cfg.control_plane {
        gateway.set_link_token(cp.token.clone());
        if let Some(key) = &cp.dp_key {
            gateway.set_dp_key(Some(key.0));
        }
        if let Some(url) = &cp.jwks_url {
            gateway.require_org_tokens(Arc::new(crate::org_token::OrgTokens::new(
                Some(url.clone()),
                Some(cp.data_plane.clone()),
            )));
        }
    }
    let _ = node.gateway.set(gateway.clone());
    let offset = node.cfg.raft_port_offset;
    let raft = if offset == 0 {
        crate::network::router(node.raft.clone(), node.catchup.clone())
    } else {
        let raft_listen = SocketAddr::new(listen.ip(), listen.port().saturating_add(offset));
        node.raft_threads.serve(
            raft_listener(raft_listen)?,
            node.raft.clone(),
            node.catchup.clone(),
        )?;
        tracing::info!(shard, node = id, %raft_listen, "raft listening");
        axum::Router::new()
    };
    let app = crate::api::router(node.clone())
        .merge(raft)
        .merge(crate::exec_api::router(node.clone()))
        .merge(crate::stream::router(node.clone()))
        .merge(crate::api_session::router(node.clone()))
        .merge(crate::api_files::router(node.clone()))
        .merge(crate::sqldb::router(node.clone()))
        .merge(crate::gateway_dp::shard_router(node.clone()))
        .merge(crate::gateway::router(gateway.clone()))
        .merge(extra(gateway))
        .layer(axum::middleware::from_fn_with_state(
            node.clone(),
            crate::catchup::guard,
        ));
    tracing::info!(shard, node = id, %listen, "node listening");
    // Decision: serve a make-service, not the router. axum 0.8 serving a
    // `Router` rebuilds its route table (a map of every route) for each
    // connection and keeps it while the connection lives: ~280 MB on a node
    // that 7,000 streams reconnected to after a peer's kill (heap profile,
    // 2026-11-23). `into_make_service` builds it once.
    let server = tokio::spawn(async move { axum::serve(listener, app.into_make_service()).await });
    if !join && let Err(error) = node.bootstrap().await {
        // Decision: a node that can't form its shard stops serving too, so
        // it reads as down rather than as a member that never elects.
        server.abort();
        return Err(error);
    }
    node.spawn_loops();
    server.await??;
    Ok(())
}

/// Raft's listener on `addr`, not yet tied to any runtime. Reusing the
/// address lets a restarted node take its port back at once.
fn raft_listener(addr: SocketAddr) -> anyhow::Result<std::net::TcpListener> {
    let socket = if addr.is_ipv4() {
        tokio::net::TcpSocket::new_v4()?
    } else {
        tokio::net::TcpSocket::new_v6()?
    };
    socket.set_reuseaddr(true)?;
    socket.bind(addr)?;
    Ok(socket.listen(1024)?.into_std()?)
}

/// How long a draining executor waits for its turns to hand off. Below the
/// pod's termination grace (`infra/k8s/dataplane` in the SaaS repository).
const DRAIN_LIMIT: Duration = Duration::from_secs(60);

/// Run an executor until stopped (SIGTERM drains it first).
pub async fn serve_executor(
    cfg: crate::executor::ExecutorConfig,
    host: Arc<dyn Host>,
    listen: Option<SocketAddr>,
) -> anyhow::Result<()> {
    let executor = crate::executor::Executor::start(cfg, host)?;
    executor.spawn();
    if let Some(listen) = listen {
        let app = crate::executor::router(executor.clone());
        let listener = tokio::net::TcpListener::bind(listen).await?;
        tracing::info!(%listen, "executor listening");
        tokio::spawn(async move { axum::serve(listener, app.into_make_service()).await });
    }
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
        _ = executor.restart_wanted.notified() => {}
    }
    tracing::info!(running = executor.running(), "draining");
    executor.drain();
    // Decision: a drain waits a bounded time. A turn that cannot hand off
    // (its shard does not answer) would hold the process forever; leaving
    // it costs that turn one step, resumed elsewhere once its claim expires.
    let limit = tokio::time::Instant::now() + DRAIN_LIMIT;
    while executor.running() > 0 && tokio::time::Instant::now() < limit {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    if executor.running() > 0 {
        tracing::warn!(running = executor.running(), "drain limit reached; leaving");
    }
    if executor.wants_restart() {
        // A distinct code, so the restart counts as the executor's own.
        std::process::exit(crate::memwatch::RESTART_EXIT);
    }
    Ok(())
}

/// Write a topology file for `shards` shards of `nodes` nodes each on
/// localhost, ports `base + 100 * shard + node`.
pub fn local_topology(shards: u32, nodes: u64, base: u16) -> Topology {
    let mut topology = Topology {
        directory_shard: 0,
        shards: Default::default(),
    };
    for shard in 0..shards {
        let members = (1..=nodes)
            .map(|node| {
                (
                    node,
                    format!("127.0.0.1:{}", base as u64 + 100 * shard as u64 + node),
                )
            })
            .collect();
        topology.shards.insert(shard, members);
    }
    topology
}

async fn inspect(topology: Topology, json: bool, what: Inspect) -> anyhow::Result<()> {
    use crate::inspect::{Inspector, render_moves, render_session, render_shards};
    let inspector = Inspector::new(topology)?;
    let out = match what {
        Inspect::Shards => {
            let reports = inspector.reports().await;
            if json {
                serde_json::to_string_pretty(&reports)?
            } else {
                render_shards(&reports)
            }
        }
        Inspect::Session { id } => {
            let session = everruns::SessionId::parse(&id)?;
            let placed = inspector.placed(session).await;
            let report = match placed {
                Some(p) => inspector.session(p.shard, session).await,
                None => None,
            };
            if json {
                serde_json::to_string_pretty(
                    &serde_json::json!({"placed": placed, "session": report}),
                )?
            } else {
                render_session(session, placed, report.as_ref())
            }
        }
        Inspect::Moves => {
            let reports = inspector.reports().await;
            let mut placed = std::collections::HashMap::new();
            for nodes in reports.values() {
                for (_, report) in nodes {
                    for m in report.iter().flat_map(|r| &r.moves) {
                        if let std::collections::hash_map::Entry::Vacant(entry) =
                            placed.entry(m.session)
                        {
                            entry.insert(inspector.placed(m.session).await);
                        }
                    }
                }
            }
            if json {
                serde_json::to_string_pretty(&reports)?
            } else {
                render_moves(&reports, &placed)
            }
        }
    };
    println!("{out}");
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_statefulset_pod_names_its_ordinal() {
        assert_eq!(super::ordinal("shard0-2"), Some(2));
        assert_eq!(super::ordinal("executors-17"), Some(17));
        assert_eq!(super::ordinal("shard0"), None);
        assert_eq!(super::ordinal("shard0-x"), None);
    }
}
