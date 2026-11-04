// Everruns API server
// Decision: Uses ServerAppBuilder for composable server setup

use clap::Parser;

use anyhow::Result;
use everruns_core::host::observability::{TelemetryConfig, init_telemetry};
use everruns_server::ServerAppBuilder;
use everruns_server::auth;
use everruns_server::server::ServerConfig;
use std::sync::Arc;

// Use mimalloc as the global allocator. For a high-concurrency Tokio service
// with heavy per-request allocation, a sharded allocator reduces fragmentation
// and improves tail latency vs. system malloc. Defined here (the bin crate
// root) so it applies only to the everruns-server binary, not library crates.
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// CLI arguments for everruns-server
#[derive(Parser)]
#[command(name = "everruns-server", about = "Everruns API server")]
struct Args {
    /// Disable automatic database migrations on startup
    #[arg(long)]
    no_migrations: bool,

    /// Probe GET /health on HTTP_ADDR and exit 0 when healthy (for container healthchecks)
    #[arg(long)]
    health_check: bool,

    /// Run the system doctor against this environment's services, print the
    /// report and exit: 1 when a check is a problem, 0 otherwise
    #[arg(long)]
    doctor: bool,

    /// With --doctor, print the report as JSON
    #[arg(long, requires = "doctor")]
    json: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    if args.health_check {
        let addr = ServerConfig::from_env().addr;
        match everruns_server::health_probe::probe(&addr, std::time::Duration::from_secs(3)) {
            Ok(()) => std::process::exit(0),
            Err(err) => {
                eprintln!("health check failed: {err}");
                std::process::exit(1);
            }
        }
    }

    if args.doctor {
        use everruns_server::domains::system::doctor::preflight;
        let report = preflight::run().await?;
        if args.json {
            println!("{}", serde_json::to_string_pretty(&report)?);
        } else {
            print!("{}", preflight::render(&report));
        }
        std::process::exit(preflight::exit_code(&report));
    }

    // Initialize telemetry (guard must stay alive for the lifetime of the application)
    let mut telemetry_config = TelemetryConfig::from_env();
    if telemetry_config.service_name == "everruns" {
        telemetry_config.service_name = "everruns-server".to_string();
    }
    if telemetry_config.log_filter.is_none() {
        telemetry_config.log_filter = Some("everruns_api=debug,tower_http=debug".to_string());
    }
    telemetry_config.service_version = Some(env!("CARGO_PKG_VERSION").to_string());
    let _telemetry_guard = init_telemetry(telemetry_config);
    // Pre-compile the embedded CPython payload (see ensure_cpython_warm).
    everruns_integrations::bashkit::tools_in_shell::ensure_cpython_warm();

    // Load server config from environment
    let mut config = ServerConfig::from_env();
    config.no_migrations = args.no_migrations;

    // Start server with built-in auth backend (OSS default)
    let auth_config = auth::AuthConfig::from_env();
    ServerAppBuilder::new(config)
        .auth(move |db, platform| {
            Arc::new(auth::BuiltinAuthBackend::new(auth_config, db, platform))
        })
        .run()
        .await
}
