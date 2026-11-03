//! Trusted operator CLI; deliberately separate from the normal server binary.
use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use everruns_trace_store::{EventQuery, Limits, Monitor, Scope, TraceStore};
use object_store::{ObjectStore, aws::AmazonS3Builder, local::LocalFileSystem, path::Path};
use sqlx::postgres::PgPoolOptions;
use std::{path::PathBuf, sync::Arc};
use uuid::Uuid;

#[derive(Parser)]
#[command(about = "Export and query analytical copies of Everruns session events")]
struct Cli {
    #[arg(long, env = "DATABASE_URL", hide_env_values = true)]
    database_url: Option<String>,
    /// Stable catalog identity; bind to one bucket/root for its entire lifetime.
    #[arg(long, env = "TRACE_STORE_NAME", default_value = "default")]
    store_name: String,
    #[arg(long, env = "TRACE_STORE_PREFIX", default_value = "traces")]
    prefix: String,
    /// Local object root, mutually exclusive with --bucket.
    #[arg(long, env = "TRACE_STORE_DIRECTORY")]
    directory: Option<PathBuf>,
    /// Uses AWS_* SDK environment configuration, including AWS_ENDPOINT.
    #[arg(long, env = "TRACE_STORE_BUCKET")]
    bucket: Option<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Args)]
struct RunArgs {
    /// Disabled by default, even when object storage is configured.
    #[arg(long,env="TRACE_STORE_ENABLED",default_value_t=false,action=clap::ArgAction::Set)]
    enabled: bool,
    #[arg(long, env = "TRACE_STORE_ORG_IDS", value_delimiter = ',')]
    org_ids: Vec<i64>,
    #[arg(long, env = "TRACE_STORE_POLL_SECONDS", default_value_t = 5)]
    poll_seconds: u64,
    #[arg(long, env = "TRACE_STORE_MAX_SESSIONS", default_value_t = 5)]
    max_sessions: usize,
    #[arg(
        long,
        env = "TRACE_STORE_METRICS_ADDR",
        default_value = "127.0.0.1:9465"
    )]
    metrics_addr: std::net::SocketAddr,
}

#[derive(Subcommand)]
enum Command {
    /// Apply only companion metadata DDL; leave application migration history intact.
    MigrateCatalog,
    /// Probe companion readiness; requires no database/object-store credentials.
    Probe {
        #[arg(long, default_value = "127.0.0.1:9465")]
        addr: std::net::SocketAddr,
    },
    /// Optional backfill, catch-up and synthetic shadow observer with metrics.
    Run(RunArgs),
    /// Snapshot fixed targets or resume a finite backfill; each pass is bounded.
    Backfill {
        #[arg(long)]
        org_id: i64,
        #[arg(long)]
        resume: Option<Uuid>,
        #[arg(long, default_value_t = 1)]
        max_passes: usize,
        #[arg(long, default_value_t = 20)]
        max_sessions: usize,
    },
    /// Read durable backfill progress without exporting additional history.
    BackfillStatus {
        #[arg(long)]
        org_id: i64,
        #[arg(long)]
        run_id: Uuid,
    },
    /// Compare a bounded shared window against PostgreSQL (synthetic workload).
    Shadow {
        #[arg(long)]
        org_id: i64,
        #[arg(long)]
        session_id: Option<Uuid>,
        #[arg(long, default_value_t = 5)]
        max_sessions: usize,
    },
    /// Current backlog and published storage sizes for one org.
    Status {
        #[arg(long)]
        org_id: i64,
    },
    /// Export one bounded batch; repeat until exported=0.
    Export {
        #[arg(long)]
        org_id: i64,
        #[arg(long)]
        session_id: Uuid,
    },
    /// Discover and export an organization's pending sessions.
    Sync {
        #[arg(long)]
        org_id: i64,
        #[arg(long, default_value_t = 20)]
        max_sessions: usize,
        /// Continue polling until interrupted.
        #[arg(long)]
        follow: bool,
        #[arg(long, default_value_t = 5)]
        poll_seconds: u64,
    },
    /// Query exported summaries; full payloads are read only by Get.
    Query {
        #[arg(long)]
        org_id: i64,
        #[arg(long)]
        session_id: Option<Uuid>,
        #[arg(long)]
        after_sequence: Option<i32>,
        #[arg(long)]
        event_type: Vec<String>,
        #[arg(long)]
        turn_id: Option<String>,
        #[arg(long)]
        from: Option<chrono::DateTime<chrono::Utc>>,
        #[arg(long)]
        until: Option<chrono::DateTime<chrono::Utc>>,
        #[arg(long, default_value_t = 100)]
        limit: usize,
    },
    /// Fetch one full checksum-verified event by identity.
    Get {
        #[arg(long)]
        org_id: i64,
        #[arg(long)]
        session_id: Uuid,
        #[arg(long)]
        event_id: Uuid,
    },
    /// Atomically merge a bounded group of session segments.
    Compact {
        #[arg(long)]
        org_id: i64,
        #[arg(long)]
        session_id: Uuid,
    },
    /// Delete the analytical copy and prevent re-export; leaves source events intact.
    Delete {
        #[arg(long)]
        org_id: i64,
        #[arg(long)]
        session_id: Uuid,
    },
    /// Remove unreferenced/retired objects; continue using next_cursor.
    Gc {
        #[arg(long)]
        after: Option<String>,
        #[arg(long, default_value_t = 1000)]
        limit: usize,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    if matches!(&cli.command, Command::MigrateCatalog) {
        let url = cli
            .database_url
            .as_ref()
            .context("DATABASE_URL is required")?;
        let pool = PgPoolOptions::new().max_connections(1).connect(url).await?;
        let applied = everruns_trace_store::migrate_catalog(&pool).await?;
        pool.close().await;
        println!(
            "{}",
            serde_json::json!({"catalog_migrations_applied":applied})
        );
        return Ok(());
    }
    if let Command::Probe { addr } = &cli.command {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            let mut stream = tokio::net::TcpStream::connect(addr).await?;
            stream
                .write_all(b"GET /readyz HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .await?;
            let mut response = Vec::new();
            stream.take(256).read_to_end(&mut response).await?;
            anyhow::ensure!(
                response.starts_with(b"HTTP/1.1 200 "),
                "companion is not ready"
            );
            Ok::<(), anyhow::Error>(())
        })
        .await
        .context("readiness probe timed out")??;
        return Ok(());
    }
    // This branch precedes every database, object-store, directory and listener
    // operation. Installing the executable does not enable optional storage.
    if matches!(&cli.command,Command::Run(args) if !args.enabled) {
        println!("{}", serde_json::json!({"enabled":false}));
        return Ok(());
    }
    if let Command::Run(args) = &cli.command {
        anyhow::ensure!(
            !args.org_ids.is_empty() && args.org_ids.len() <= 100,
            "enabled companion requires explicit org ids"
        );
        anyhow::ensure!(
            (1..=100).contains(&args.max_sessions) && (1..=3600).contains(&args.poll_seconds),
            "invalid companion budgets"
        );
    }
    anyhow::ensure!(
        !(cli.directory.is_some() && cli.bucket.is_some()),
        "select a directory or bucket, not both"
    );
    let database_url = cli
        .database_url
        .context("DATABASE_URL is required for enabled operations")?;
    let objects: Arc<dyn ObjectStore> = if let Some(root) = cli.directory {
        std::fs::create_dir_all(&root)?;
        Arc::new(LocalFileSystem::new_with_prefix(root)?)
    } else {
        Arc::new(
            AmazonS3Builder::from_env()
                .with_bucket_name(cli.bucket.context("bucket required")?)
                .build()?,
        )
    };
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .connect(&database_url)
        .await?;
    let store = TraceStore::new(
        pool.clone(),
        objects,
        cli.store_name,
        Path::parse(cli.prefix)?,
        Limits::default(),
    )?;
    let output = match cli.command {
        Command::MigrateCatalog => unreachable!("handled before object initialization"),
        Command::Probe { .. } => unreachable!("handled before storage initialization"),
        Command::Run(args) => run_companion(store, args).await?,
        Command::Backfill {
            org_id,
            resume,
            max_passes,
            max_sessions,
        } => {
            anyhow::ensure!(
                (1..=10000).contains(&max_passes),
                "max-passes must be 1..=10000"
            );
            let scope = Scope::new(org_id)?;
            let mut status = if let Some(id) = resume {
                store.backfill_status(scope, id).await?
            } else {
                store.start_backfill(scope).await?
            };
            for _ in 0..max_passes {
                if status.complete() {
                    break;
                }
                status = store
                    .advance_backfill(scope, status.run_id, max_sessions)
                    .await?;
            }
            serde_json::to_value(status)?
        }
        Command::BackfillStatus { org_id, run_id } => {
            serde_json::to_value(store.backfill_status(Scope::new(org_id)?, run_id).await?)?
        }
        Command::Shadow {
            org_id,
            session_id,
            max_sessions,
        } => {
            let scope = Scope::new(org_id)?;
            if let Some(id) = session_id {
                serde_json::to_value(store.shadow_session(scope, id, 100).await?)?
            } else {
                serde_json::to_value(store.shadow_once(scope, max_sessions).await?)?
            }
        }
        Command::Status { org_id } => {
            serde_json::to_value(store.progress(Scope::new(org_id)?).await?)?
        }
        Command::Export { org_id, session_id } => {
            serde_json::json!({"exported":store.export_session(Scope::new(org_id)?,session_id).await?})
        }
        Command::Sync {
            org_id,
            max_sessions,
            follow,
            poll_seconds,
        } => {
            anyhow::ensure!(
                (1..=3600).contains(&poll_seconds),
                "poll interval must be 1..=3600 seconds"
            );
            let scope = Scope::new(org_id)?;
            loop {
                let exported = store.sync_once(scope, max_sessions).await?;
                if !follow {
                    break serde_json::json!({"exported":exported});
                }
                println!("{}", serde_json::json!({"exported":exported}));
                tokio::select! {
                    result=tokio::signal::ctrl_c()=> {result?;break serde_json::json!({"stopped":true});},
                    _=tokio::time::sleep(std::time::Duration::from_secs(poll_seconds))=>{},
                }
            }
        }
        Command::Query {
            org_id,
            session_id,
            after_sequence,
            event_type,
            turn_id,
            from,
            until,
            limit,
        } => serde_json::to_value(
            store
                .query(
                    Scope::new(org_id)?,
                    &EventQuery {
                        session_id,
                        after_sequence,
                        through_sequence: None,
                        event_types: event_type,
                        turn_id,
                        from,
                        until,
                        limit,
                    },
                )
                .await?,
        )?,
        Command::Get {
            org_id,
            session_id,
            event_id,
        } => serde_json::to_value(
            store
                .read_event(Scope::new(org_id)?, session_id, event_id)
                .await?,
        )?,
        Command::Compact { org_id, session_id } => {
            serde_json::json!({"segments_compacted":store.compact_session(Scope::new(org_id)?,session_id).await?})
        }
        Command::Delete { org_id, session_id } => {
            store
                .delete_session(Scope::new(org_id)?, session_id)
                .await?;
            serde_json::json!({"deleted":true})
        }
        Command::Gc { after, limit } => {
            let (deleted, next_cursor) = store.collect_garbage(after.as_deref(), limit).await?;
            serde_json::json!({"objects_deleted":deleted,"next_cursor":next_cursor})
        }
    };
    println!("{}", serde_json::to_string(&output)?);
    pool.close().await;
    Ok(())
}

async fn run_companion(store: TraceStore, args: RunArgs) -> Result<serde_json::Value> {
    use std::{future::IntoFuture, time::Duration};
    let scopes = args
        .org_ids
        .into_iter()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .map(Scope::new)
        .collect::<Result<Vec<_>>>()?;
    let monitor = Monitor::default();
    let listener = tokio::net::TcpListener::bind(args.metrics_addr).await?;
    let health_age = Duration::from_secs(args.poll_seconds.saturating_mul(3).max(60));
    let http = axum::serve(listener, monitor.router(health_age)).into_future();
    let observe = async {
        loop {
            let report = store.parallel_cycle(&scopes, args.max_sessions).await?;
            monitor.record(&report);
            println!("{}", serde_json::to_string(&report)?);
            tokio::time::sleep(Duration::from_secs(args.poll_seconds)).await;
        }
        #[allow(unreachable_code)]
        Ok::<(), anyhow::Error>(())
    };
    tokio::select! {
        result=http=>{result?;anyhow::bail!("metrics listener stopped");},
        result=observe=>{result?;},
        result=shutdown_signal()=>{result?;},
    }
    Ok(serde_json::json!({"stopped":true}))
}

async fn shutdown_signal() -> Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {r=tokio::signal::ctrl_c()=>{r?;},_=terminate.recv()=>{}}
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await?;
    Ok(())
}
