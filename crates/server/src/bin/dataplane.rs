// Operator CLI for data planes (data plane v2 step 7): register a data
// plane, put an org on one, suspend or resume it, and the restore steps
// (config diff, epoch bump). Needs the database, so only operators run it.
//
// Run with: cargo run -p everruns-server --bin everruns-dataplane -- --help

use std::io::Read as _;
use std::sync::Arc;

use anyhow::{Context, Result};
use everruns_contracts::dataplane::OrgState;
use everruns_server::domains::data_plane::{self, ControlPlane, Link};
use everruns_server::storage::{Database, EncryptionService, StorageBackend};

const HELP: &str = r#"
everruns-dataplane - operate data planes (v2)

USAGE:
    everruns-dataplane register <data_plane_id> <gateway_url> [<admin_url>]
    everruns-dataplane enable <org_public_id> <data_plane_id>
    everruns-dataplane suspend <org_public_id> [<reason>]
    everruns-dataplane resume <org_public_id>
    everruns-dataplane secret <org_public_id> <secret_id>     (value on stdin)
    everruns-dataplane status <org_public_id>
    everruns-dataplane diff <org_public_id>
    everruns-dataplane bump-epoch

`enable` switches an org to a data plane: its new sessions run there. After a
database restore, run `diff` for every v2 org, resolve what it lists, then
`bump-epoch`: every data plane then takes a full snapshot.

ENVIRONMENT:
    DATABASE_URL              PostgreSQL connection string (required)
    SECRETS_ENCRYPTION_KEY    The server's secrets key (stores org data keys)
    EVERRUNS_DP_KEY           The data plane's wrapping key (base64, 32 bytes)
    EVERRUNS_DP_TOKEN         The link's bearer token
"#;

fn org(arg: Option<&String>) -> Result<i64> {
    let public = arg.context("an org public id is required")?;
    data_plane::org_internal(public).with_context(|| format!("{public} is not an org id"))
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(command) = args.first().map(String::as_str) else {
        eprintln!("{HELP}");
        std::process::exit(2);
    };
    if matches!(command, "-h" | "--help" | "help") {
        println!("{HELP}");
        return Ok(());
    }
    let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL is not set")?;
    let pool = sqlx::PgPool::connect(&database_url)
        .await
        .context("could not connect to the database")?;
    let db = Arc::new(StorageBackend::from_database(Database::new(pool)));
    let encryption = EncryptionService::from_env().ok().map(Arc::new);
    let cp = ControlPlane::new(db.clone(), encryption, Link::from_env()?);
    match command {
        "register" => {
            let id = args.get(1).context("a data plane id is required")?;
            let url = args.get(2).context("the gateway URL is required")?;
            let admin = args.get(3).unwrap_or(url);
            db.dp_upsert_data_plane(id, url.trim_end_matches('/'), admin.trim_end_matches('/'))
                .await?;
            println!("data plane {id} at {url}");
        }
        "enable" => {
            let org_id = org(args.get(1))?;
            let plane = args.get(2).context("a data plane id is required")?;
            let row = data_plane::enable_org(&cp, org_id, plane).await?;
            println!(
                "{} runs on {} (config version {})",
                data_plane::org_public(org_id),
                row.data_plane_id,
                row.version
            );
        }
        "suspend" | "resume" => {
            let org_id = org(args.get(1))?;
            let state = if command == "suspend" {
                OrgState::Suspended
            } else {
                OrgState::Active
            };
            data_plane::set_org_state(&cp, org_id, state, args.get(2).cloned()).await?;
            println!("{command}ed {}", data_plane::org_public(org_id));
        }
        "secret" => {
            let org_id = org(args.get(1))?;
            let secret_id = args.get(2).context("a secret id is required")?;
            let mut value = Vec::new();
            std::io::stdin().read_to_end(&mut value)?;
            let revision = data_plane::put_secret(&cp, org_id, secret_id, &value).await?;
            println!("secret {secret_id} revision {revision}");
        }
        "status" => {
            let org_id = org(args.get(1))?;
            match db.dp_org(org_id).await? {
                None => println!(
                    "{} runs on the platform's workers (v1)",
                    data_plane::org_public(org_id)
                ),
                Some(row) => println!(
                    "{}\n  data plane: {}\n  enabled: {}\n  state: {}\n  config: version {} (delivered {} in epoch {})\n  attempts: {}",
                    data_plane::org_public(org_id),
                    row.data_plane_id,
                    row.enabled,
                    row.state,
                    row.version,
                    row.delivered_version,
                    row.delivered_epoch,
                    row.attempts,
                ),
            }
        }
        "diff" => {
            let org_id = org(args.get(1))?;
            let diff = data_plane::config_diff(&cp, org_id).await?;
            println!("{}", serde_json::to_string_pretty(&diff)?);
        }
        "bump-epoch" => {
            let epoch = db.dp_bump_epoch().await?;
            println!("control plane epoch {epoch}: every v2 org gets a full snapshot");
        }
        other => {
            eprintln!("unknown command {other}\n{HELP}");
            std::process::exit(2);
        }
    }
    Ok(())
}
