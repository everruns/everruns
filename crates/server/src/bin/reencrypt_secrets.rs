// CLI tool for re-encrypting secrets after key rotation.
// Run with: cargo run --bin reencrypt-secrets -- --help

use anyhow::{Context, Result};
use everruns_server::storage::encryption_rotation::process_table;
use everruns_server::storage::{ENCRYPTED_COLUMNS, EncryptedColumn, EncryptionService};
use sqlx::PgPool;
use std::env;

#[derive(Debug)]
struct Args {
    dry_run: bool,
    batch_size: i64,
    table: Option<String>,
}

impl Args {
    fn parse() -> Result<Self> {
        let args: Vec<String> = env::args().collect();
        let mut dry_run = false;
        let mut batch_size = 100i64;
        let mut table = None;
        let mut i = 1;

        while i < args.len() {
            match args[i].as_str() {
                "--dry-run" | "-n" => dry_run = true,
                "--batch-size" | "-b" => {
                    i += 1;
                    batch_size = args
                        .get(i)
                        .context("--batch-size requires a value")?
                        .parse()
                        .context("Invalid batch size")?;
                }
                "--table" | "-t" => {
                    i += 1;
                    table = Some(args.get(i).context("--table requires a value")?.to_string());
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                arg => {
                    eprintln!("Unknown argument: {}", arg);
                    print_help();
                    std::process::exit(1);
                }
            }
            i += 1;
        }

        Ok(Self {
            dry_run,
            batch_size,
            table,
        })
    }
}

fn print_help() {
    eprintln!(
        r#"
reencrypt-secrets - Re-encrypt database secrets after key rotation

USAGE:
    reencrypt-secrets [OPTIONS]

OPTIONS:
    -n, --dry-run           Show what would be changed without making changes
    -b, --batch-size <N>    Process N records at a time (default: 100)
    -t, --table <NAME>      Only process specified table (default: all)
    -h, --help              Show this help message

ENVIRONMENT:
    DATABASE_URL                    PostgreSQL connection string (required)
    SECRETS_ENCRYPTION_KEY          Current encryption key (required)
    SECRETS_ENCRYPTION_KEY_PREVIOUS Previous encryption key for rotation

EXAMPLES:
    # Dry run to see what would be re-encrypted
    reencrypt-secrets --dry-run

    # Re-encrypt all tables
    reencrypt-secrets

    # Re-encrypt specific table with smaller batches
    reencrypt-secrets --table providers --batch-size 50
"#
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("reencrypt_secrets=info".parse()?)
                .add_directive("everruns_server::storage::encryption_rotation=info".parse()?),
        )
        .init();

    let args = Args::parse()?;

    // Load environment
    if let Ok(path) = dotenvy::dotenv() {
        tracing::info!("Loaded .env from {:?}", path);
    }

    // Initialize encryption service
    let encryption = EncryptionService::from_env().context(
        "Failed to initialize encryption service. Ensure SECRETS_ENCRYPTION_KEY is set.",
    )?;

    tracing::info!(
        "Encryption service initialized. Primary key: {}",
        encryption.primary_key_id()
    );
    tracing::info!("Available keys: {:?}", encryption.available_key_ids());

    // Connect to database
    let database_url = env::var("DATABASE_URL").context("DATABASE_URL not set")?;
    let pool = PgPool::connect(&database_url)
        .await
        .context("Failed to connect to database")?;

    tracing::info!("Connected to database");

    // Get list of tables to process
    let tables = get_encrypted_tables(&args.table);

    if tables.is_empty() {
        tracing::info!("No tables with encrypted fields found");
        return Ok(());
    }

    let mut total_processed = 0u64;
    let mut total_reencrypted = 0u64;

    for table_info in tables {
        if args.table.is_some() && args.table.as_deref() != Some(table_info.table) {
            continue;
        }

        tracing::info!("Processing table: {}", table_info.table);

        let (processed, reencrypted) = process_table(
            &pool,
            &encryption,
            &table_info,
            args.batch_size,
            args.dry_run,
        )
        .await?;

        total_processed += processed;
        total_reencrypted += reencrypted;
    }

    if args.dry_run {
        tracing::info!(
            "DRY RUN: Would re-encrypt {} of {} records",
            total_reencrypted,
            total_processed
        );
    } else {
        tracing::info!(
            "Re-encrypted {} of {} records",
            total_reencrypted,
            total_processed
        );
    }

    Ok(())
}

/// Select registered encrypted columns; table names never come from SQL input.
fn get_encrypted_tables(filter: &Option<String>) -> Vec<EncryptedColumn> {
    ENCRYPTED_COLUMNS
        .iter()
        .filter(|column| filter.as_deref().is_none_or(|name| column.table == name))
        .cloned()
        .collect()
}
