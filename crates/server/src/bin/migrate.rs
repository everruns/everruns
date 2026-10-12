// Migrate a database to the current schema, the way server startup does,
// including adopting the migration baseline on a database that ran the legacy
// history (crates/server/src/setup/migrations.rs). The admin container's
// `migrate` command runs this; plain `sqlx migrate run` cannot adopt.
// Run with: DATABASE_URL=... cargo run --bin everruns-migrate

use anyhow::{Context, Result};
use sqlx::PgPool;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("everruns_server=info".parse().unwrap()),
        )
        .init();

    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL not set")?;
    let pool = PgPool::connect(&database_url)
        .await
        .context("Failed to connect to database")?;
    everruns_server::setup::migrations::run(&pool).await?;
    println!("Migrations complete");
    Ok(())
}
