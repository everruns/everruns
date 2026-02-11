// Everruns API server
// Decision: Thin binary — delegates to server::run() for reuse by SaaS binary

use clap::Parser;

use anyhow::Result;
use everruns_core::telemetry::{TelemetryConfig, init_telemetry};
use everruns_server::auth;
use everruns_server::server::{ServerConfig, run};
use std::sync::Arc;

/// CLI arguments for everruns-server
#[derive(Parser)]
#[command(name = "everruns-server", about = "Everruns API server")]
struct Args {
    /// Disable automatic database migrations on startup
    #[arg(long)]
    no_migrations: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

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

    // Load server config from environment
    let mut config = ServerConfig::from_env();
    config.no_migrations = args.no_migrations;

    // Start server with built-in auth backend (OSS default)
    let auth_config = auth::AuthConfig::from_env();
    run(
        config,
        |db| Arc::new(auth::BuiltinAuthBackend::new(auth_config, db)),
        None,
    )
    .await
}
