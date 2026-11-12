//! The cell node command line, for a node binary that brings its host
//! (model providers and tools; agents come from clients).
//!
//! ```text
//! app node    --topology cluster.json --cell 0 --node 1 --data .cell/c0n1
//! app gateway --topology cluster.json --listen 127.0.0.1:7000
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
use crate::state::CellId;
use crate::types::NodeId;

#[derive(Parser)]
#[command(about = "A Raft-replicated cell node for Everruns agents")]
struct Cli {
    #[command(subcommand)]
    command: Mode,
}

#[derive(Subcommand)]
enum Mode {
    /// Run one node of a cell (plus the gateway API).
    Node {
        #[arg(long, env = "CELL_TOPOLOGY")]
        topology: PathBuf,
        #[arg(long)]
        cell: CellId,
        #[arg(long)]
        node: NodeId,
        #[arg(long)]
        data: PathBuf,
        /// Object store for session history and snapshot copies, shared by
        /// the whole fleet: `s3://bucket/prefix` (with `AWS_*` settings) or
        /// `file:///dir`.
        #[arg(long, env = "CELL_STORE")]
        store: String,
        /// Unload idle sessions when resident events pass this many MiB.
        #[arg(long, env = "CELL_MEMORY_BUDGET_MB", default_value = "1024")]
        memory_budget_mb: u64,
        /// Also unload sessions idle this many seconds (0: only under pressure).
        #[arg(long, env = "CELL_UNLOAD_IDLE_SECS", default_value = "0")]
        unload_idle_secs: u64,
        #[arg(long, default_value = "5000")]
        claim_ttl_ms: u64,
        /// Turns one node runs at once; more wait in their session's inbox.
        #[arg(long, env = "CELL_MAX_TURNS_PER_NODE", default_value = "256")]
        max_turns_per_node: usize,
        #[arg(long, default_value = "20000")]
        snapshot_every: u64,
        /// Log entries kept behind each snapshot, for followers that fell
        /// behind; further back they catch up by snapshot. The kept entries
        /// stay in memory (see `node.rs` for the measured cost).
        #[arg(long, default_value = "256")]
        keep_log: u64,
        /// Join a running cell instead of forming it: the node waits until
        /// the cell's leader adds it (`POST /v1/cells/{cell}/nodes`).
        #[arg(long)]
        join: bool,
        /// Address to serve on; required with `--join` when the topology
        /// file does not list this node.
        #[arg(long)]
        listen: Option<String>,
        /// Leave turns to executors outside the cell, which pull them with
        /// credits; at most this many run in the cell at once (0: nodes run
        /// turns themselves).
        #[arg(long, env = "CELL_EXECUTOR_CAP", default_value = "0")]
        executor_cap: usize,
        /// The control plane's `/dp/v1` base URL: the leader delivers the
        /// feed there and asks it for spend allowances.
        #[arg(long, env = "CELL_CONTROL_PLANE")]
        control_plane: Option<String>,
    },
    /// Run an executor: pull turns from every cell and run them.
    Executor {
        #[arg(long, env = "CELL_TOPOLOGY")]
        topology: PathBuf,
        /// This executor's id (unique in the fleet; at least 2^32).
        #[arg(long)]
        id: u64,
        #[arg(long, env = "CELL_STORE")]
        store: String,
        /// Turns this executor runs at once.
        #[arg(long, env = "CELL_EXECUTOR_SLOTS", default_value = "64")]
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
    },
    /// Run the edge router: one public URL for the control plane and the
    /// data plane (`edge.rs`).
    Edge {
        #[arg(long, env = "CELL_TOPOLOGY")]
        topology: PathBuf,
        #[arg(long, env = "CELL_EDGE_LISTEN")]
        listen: SocketAddr,
        /// The control plane's base URL (its `/v1` API).
        #[arg(long, env = "CELL_EDGE_CONTROL_PLANE")]
        control_plane: String,
        /// The control plane's org token keys; its
        /// `/.well-known/jwks.json` when unset.
        #[arg(long, env = "CELL_JWKS_URL")]
        jwks_url: Option<String>,
        /// The data plane org tokens must name.
        #[arg(long, env = "CELL_DATA_PLANE")]
        data_plane: Option<String>,
        /// The path the API is served under (`/api` behind the local
        /// proxy); empty when it is served at the root.
        #[arg(long, env = "CELL_EDGE_API_PREFIX", default_value = "")]
        api_prefix: String,
    },
    /// Run a stateless gateway.
    Gateway {
        #[arg(long, env = "CELL_TOPOLOGY")]
        topology: PathBuf,
        #[arg(long)]
        listen: SocketAddr,
    },
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
    match Cli::parse().command {
        Mode::Node {
            topology,
            cell,
            node,
            data,
            store,
            memory_budget_mb,
            unload_idle_secs,
            claim_ttl_ms,
            max_turns_per_node,
            snapshot_every,
            keep_log,
            join,
            listen,
            executor_cap,
            control_plane,
        } => {
            let source = TopologySource { path: topology };
            let seeded = source.read()?.cells.get(&cell).cloned();
            let members = match (&seeded, join) {
                (Some(members), false) => members.clone(),
                (None, false) => anyhow::bail!("cell {cell} not in topology (use --join)"),
                (_, true) => Default::default(),
            };
            let addr = listen
                .or_else(|| seeded.and_then(|m| m.get(&node).cloned()))
                .ok_or_else(|| anyhow::anyhow!("node {node}: no address (use --listen)"))?;
            let cfg = NodeConfig {
                cell,
                node_id: node,
                members,
                data_dir: data,
                store: crate::store::Store::open(&store)?,
                memory_budget: memory_budget_mb * 1024 * 1024,
                unload_idle: (unload_idle_secs > 0).then(|| Duration::from_secs(unload_idle_secs)),
                claim_ttl: Duration::from_millis(claim_ttl_ms),
                max_turns_per_node,
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
        } => {
            let cfg = crate::executor::ExecutorConfig {
                id,
                topology: TopologySource { path: topology },
                store: crate::store::Store::open(&store)?.with_cache(cache_mb << 20),
                slots,
                mirror_sessions,
                pull_wait: Duration::from_secs(1),
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
            axum::serve(listener, edge.router()).await?;
            Ok(())
        }
        Mode::Gateway { topology, listen } => {
            let gateway = Gateway::new(TopologySource { path: topology });
            let listener = tokio::net::TcpListener::bind(listen).await?;
            tracing::info!(%listen, "gateway listening");
            axum::serve(listener, crate::gateway::router(gateway)).await?;
            Ok(())
        }
    }
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
    let cell = cfg.cell;
    let id = cfg.node_id;
    let listen = listener.local_addr()?;
    let node = Node::start(cfg, host).await?;
    let gateway = Gateway::new(topology);
    if let Some(cp) = &node.cfg.control_plane {
        gateway.set_link_token(cp.token.clone());
        if let Some(url) = &cp.jwks_url {
            gateway.require_org_tokens(Arc::new(crate::org_token::OrgTokens::new(
                Some(url.clone()),
                Some(cp.data_plane.clone()),
            )));
        }
    }
    let _ = node.gateway.set(gateway.clone());
    let app = crate::api::router(node.clone())
        .merge(crate::exec_api::router(node.clone()))
        .merge(crate::stream::router(node.clone()))
        .merge(crate::api_session::router(node.clone()))
        .merge(crate::gateway_dp::cell_router(node.clone()))
        .merge(crate::gateway::router(gateway));
    tracing::info!(cell, node = id, %listen, "node listening");
    let server = tokio::spawn(async move { axum::serve(listener, app).await });
    if !join && let Err(error) = node.bootstrap().await {
        // Decision: a node that can't form its cell stops serving too, so
        // it reads as down rather than as a member that never elects.
        server.abort();
        return Err(error);
    }
    node.spawn_loops();
    server.await??;
    Ok(())
}

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
        tokio::spawn(async move { axum::serve(listener, app).await });
    }
    let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! {
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
    tracing::info!(running = executor.running(), "draining");
    executor.drain();
    while executor.running() > 0 {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(())
}

/// Write a topology file for `cells` cells of `nodes` nodes each on
/// localhost, ports `base + 100 * cell + node`.
pub fn local_topology(cells: u32, nodes: u64, base: u16) -> Topology {
    let mut topology = Topology {
        directory_cell: 0,
        cells: Default::default(),
    };
    for cell in 0..cells {
        let members = (1..=nodes)
            .map(|node| {
                (
                    node,
                    format!("127.0.0.1:{}", base as u64 + 100 * cell as u64 + node),
                )
            })
            .collect();
        topology.cells.insert(cell, members);
    }
    topology
}
