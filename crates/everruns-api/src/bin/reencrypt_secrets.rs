// CLI tool for re-encrypting secrets after key rotation.
// Run with: cargo run --bin reencrypt-secrets -- --help

use anyhow::{Context, Result};
use everruns_storage::{EncryptionService, ENCRYPTED_COLUMNS};
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
    reencrypt-secrets --table llm_providers --batch-size 50
"#
    );
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("reencrypt_secrets=info".parse().unwrap()),
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
        if args.table.is_some() && args.table.as_deref() != Some(table_info.name) {
            continue;
        }

        tracing::info!("Processing table: {}", table_info.name);

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

/// Table metadata for encrypted fields (derived from ENCRYPTED_COLUMNS registry)
struct EncryptedTable {
    name: &'static str,
    id_column: &'static str,
    column: &'static str,
}

/// Returns list of tables with encrypted fields from the central registry.
/// The registry is defined in everruns_storage::ENCRYPTED_COLUMNS.
/// A test in encryption.rs ensures all encrypted columns in migrations are registered.
fn get_encrypted_tables(filter: &Option<String>) -> Vec<EncryptedTable> {
    let all_tables: Vec<EncryptedTable> = ENCRYPTED_COLUMNS
        .iter()
        .map(|ec| EncryptedTable {
            name: ec.table,
            id_column: ec.id_column,
            column: ec.column,
        })
        .collect();

    if let Some(table_name) = filter {
        all_tables
            .into_iter()
            .filter(|t| t.name == table_name)
            .collect()
    } else {
        all_tables
    }
}

/// Process a single table, re-encrypting any records with old keys
async fn process_table(
    pool: &PgPool,
    encryption: &EncryptionService,
    table: &EncryptedTable,
    batch_size: i64,
    dry_run: bool,
) -> Result<(u64, u64)> {
    let mut processed = 0u64;
    let mut reencrypted = 0u64;
    let mut offset = 0i64;

    loop {
        // Fetch batch
        let query = format!(
            "SELECT {}, {} FROM {} ORDER BY {} LIMIT {} OFFSET {}",
            table.id_column, table.column, table.name, table.id_column, batch_size, offset
        );

        let rows: Vec<(uuid::Uuid, Option<Vec<u8>>)> = sqlx::query_as(&query)
            .fetch_all(pool)
            .await
            .context("Failed to fetch records")?;

        if rows.is_empty() {
            break;
        }

        for (id, encrypted_data) in &rows {
            processed += 1;

            if let Some(data) = encrypted_data {
                // Check if needs re-encryption
                match encryption.is_current_key(data) {
                    Ok(true) => {
                        // Already using current key
                    }
                    Ok(false) => {
                        // Needs re-encryption
                        let key_id = EncryptionService::get_key_id(data).unwrap_or_default();
                        if dry_run {
                            tracing::info!(
                                "Would re-encrypt {}.{} (id={}, current_key={})",
                                table.name,
                                table.column,
                                id,
                                key_id
                            );
                        } else {
                            // Decrypt and re-encrypt
                            match encryption.reencrypt(data) {
                                Ok(Some(new_data)) => {
                                    // Update record
                                    let update_query = format!(
                                        "UPDATE {} SET {} = $1 WHERE {} = $2",
                                        table.name, table.column, table.id_column
                                    );
                                    sqlx::query(&update_query)
                                        .bind(&new_data)
                                        .bind(id)
                                        .execute(pool)
                                        .await
                                        .context("Failed to update record")?;

                                    tracing::info!(
                                        "Re-encrypted {}.{} (id={}, {} -> {})",
                                        table.name,
                                        table.column,
                                        id,
                                        key_id,
                                        encryption.primary_key_id()
                                    );
                                }
                                Ok(None) => {
                                    // Already current (shouldn't happen due to is_current_key check)
                                }
                                Err(e) => {
                                    tracing::error!(
                                        "Failed to re-encrypt {}.{} (id={}): {}",
                                        table.name,
                                        table.column,
                                        id,
                                        e
                                    );
                                    continue;
                                }
                            }
                        }
                        reencrypted += 1;
                    }
                    Err(e) => {
                        tracing::warn!(
                            "Failed to check key for {}.{} (id={}): {}",
                            table.name,
                            table.column,
                            id,
                            e
                        );
                    }
                }
            }
        }

        offset += batch_size;

        // Progress log every 1000 records
        if processed % 1000 == 0 {
            tracing::info!(
                "Progress: {} processed, {} need re-encryption",
                processed,
                reencrypted
            );
        }
    }

    Ok((processed, reencrypted))
}
