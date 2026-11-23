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
use everruns_server::storage::{Database, EncryptionService, ReturnToV1, StorageBackend};

const HELP: &str = r#"
everruns-dataplane - operate data planes (v2)

USAGE:
    everruns-dataplane register <data_plane_id> <gateway_url> [<admin_url>]
    everruns-dataplane enable <org_public_id> <data_plane_id>
    everruns-dataplane suspend <org_public_id> [<reason>]
    everruns-dataplane resume <org_public_id>
    everruns-dataplane secret <org_public_id> <secret_id>     (value on stdin)
    everruns-dataplane status <org_public_id>
    everruns-dataplane warehouse <org_public_id> on|off
    everruns-dataplane diff <org_public_id>
    everruns-dataplane bump-epoch
    everruns-dataplane rejoin <data_plane_id>
    everruns-dataplane reconcile <org_public_id> [--keep | --drop]
    everruns-dataplane move <org_public_id> [<session_id> | --archived]
    everruns-dataplane move-orgs <data_plane_id> <org_public_id>... [--archived]
    everruns-dataplane to-v1 <data_plane_id> [<org_public_id>...]

`enable` switches an org to a data plane: its new sessions run there. After a
database restore, run `diff` for every v2 org, resolve what it lists, then
`bump-epoch`: every data plane then takes a full snapshot. What a snapshot
lacked the data plane sets aside: `reconcile` lists it, `--keep` sends it
again as config, `--drop` forgets it. Running turns finish either way.

`to-v1` sends orgs on a data plane back to the platform's own workers (v1):
the named ones, or every org on it. Only an org that never ran a session
there goes back; one that did is listed with its session count and left
alone, since nothing moves a session from v2 back to v1. `enable` puts an org
back on a data plane later.

`rejoin` is for a data plane started over empty (new disks, new store
prefix): every org on it is enabled again and sent a full snapshot, since
the control plane cannot tell that the data plane lost what it had. A data
plane restored from its store keeps its orgs and needs no rejoin.

`move` moves a running v1 session onto the org's data plane (the org must be
enabled), keeping its id: its conversation, values, secrets and files go to a
shard and it runs there from then on. Without a session it moves every v1
session of the org (`--archived` includes archived ones), one at a time. A
session running a turn is skipped; run `move` again later. What a move left
behind is in its report.

`move-orgs` moves a batch of orgs, one after another: each v1 org is enabled
on the data plane, then every v1 session of it moves as with `move`. Name the
orgs, or `-` to read them from stdin, one per line (a batch from a list kept
elsewhere). An org on another data plane, or one that fails, does not stop the
batch; it prints one line per org and exits non-zero if any org or session
did not move. Running it again picks up what was skipped or failed.

`warehouse on` exports every event of the org's sessions as hourly gzipped
NDJSON files under `warehouse/<org>/date=.../hour=.../` in the control
plane's object storage (STORAGE_BLOB_BACKEND=s3).

ENVIRONMENT:
    DATABASE_URL              PostgreSQL connection string (required)
    SECRETS_ENCRYPTION_KEY    The server's secrets key (stores org data keys)
    EVERRUNS_DP_KEY           The data plane's wrapping key (base64, 32 bytes)
    EVERRUNS_DP_TOKEN         The link's bearer token
    STORAGE_BLOB_BACKEND      `s3` for the warehouse export (with the S3 settings)
"#;

/// The org an operator names: the public id people see (console, API), or
/// the link's id derived from the internal one (`org_{internal:032x}`).
/// Organizations created by sign-up have random public ids, so the link's
/// form alone would leave the CLI unable to name them.
async fn org(db: &StorageBackend, arg: Option<&String>) -> Result<i64> {
    let public = arg.context("an org public id is required")?;
    if let Some(row) = db.get_organization_by_public_id(public).await? {
        return Ok(row.org_id);
    }
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
    let db = Arc::new(
        StorageBackend::from_database(Database::new(pool))
            .with_blob_store(everruns_server::storage::blob_store::blob_store_from_env()?),
    );
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
            let org_id = org(&db, args.get(1)).await?;
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
            let org_id = org(&db, args.get(1)).await?;
            let state = if command == "suspend" {
                OrgState::Suspended
            } else {
                OrgState::Active
            };
            data_plane::set_org_state(&cp, org_id, state, args.get(2).cloned()).await?;
            println!("{command}ed {}", data_plane::org_public(org_id));
        }
        "secret" => {
            let org_id = org(&db, args.get(1)).await?;
            let secret_id = args.get(2).context("a secret id is required")?;
            let mut value = Vec::new();
            std::io::stdin().read_to_end(&mut value)?;
            let revision = data_plane::put_secret(&cp, org_id, secret_id, &value).await?;
            println!("secret {secret_id} revision {revision}");
        }
        "status" => {
            let org_id = org(&db, args.get(1)).await?;
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
        "warehouse" => {
            let org_id = org(&db, args.get(1)).await?;
            let on = match args.get(2).map(String::as_str) {
                Some("on") => true,
                Some("off") => false,
                _ => anyhow::bail!("warehouse <org> on|off"),
            };
            data_plane::warehouse::set(&db, org_id, on).await?;
            println!(
                "warehouse export {} for {}",
                if on { "on" } else { "off" },
                data_plane::org_public(org_id)
            );
        }
        "diff" => {
            let org_id = org(&db, args.get(1)).await?;
            let diff = data_plane::config_diff(&cp, org_id).await?;
            println!("{}", serde_json::to_string_pretty(&diff)?);
        }
        "reconcile" => {
            let org_id = org(&db, args.get(1)).await?;
            let keep = match args.get(2).map(String::as_str) {
                None => None,
                Some("--keep") => Some(true),
                Some("--drop") => Some(false),
                Some(other) => anyhow::bail!("unknown option {other}: --keep or --drop"),
            };
            let found = data_plane::reconcile(&cp, org_id, keep).await?;
            println!("{}", serde_json::to_string_pretty(&found)?);
            match (found.is_some(), keep) {
                (false, _) => eprintln!("nothing set aside"),
                (true, None) => eprintln!("set aside; resolve with --keep or --drop"),
                (true, Some(true)) => eprintln!("kept: the next delivery sends it again"),
                (true, Some(false)) => eprintln!("dropped"),
            }
        }
        "bump-epoch" => {
            let epoch = db.dp_bump_epoch().await?;
            println!("control plane epoch {epoch}: every v2 org gets a full snapshot");
        }
        "rejoin" => {
            let plane = args.get(1).context("a data plane id is required")?;
            anyhow::ensure!(
                db.dp_data_plane(plane).await?.is_some(),
                "data plane {plane} is not registered"
            );
            let orgs = db.dp_rejoin(plane).await?;
            println!("{plane}: {orgs} orgs enabled again, each gets a full snapshot");
        }
        "move" => {
            let org_id = org(&db, args.get(1)).await?;
            let drain = std::time::Duration::from_secs(120);
            let outcomes = match args.get(2).map(String::as_str) {
                Some(session) if session != "--archived" => {
                    let session = session
                        .parse()
                        .map_err(|_| anyhow::anyhow!("{session} is not a session id"))?;
                    let outcome =
                        data_plane::moves::move_session(&cp, org_id, session, drain).await;
                    vec![(session, outcome)]
                }
                flag => {
                    let archived = flag == Some("--archived");
                    data_plane::moves::move_org(&cp, org_id, archived, drain).await?
                }
            };
            let mut failed = 0;
            for (session, outcome) in outcomes {
                match outcome {
                    Ok(data_plane::moves::Moved::Done(report)) => {
                        println!("{session} moved: {}", serde_json::to_string(&report)?);
                    }
                    Ok(data_plane::moves::Moved::Skipped(reason)) => {
                        println!("{session} skipped: {reason}");
                    }
                    Err(error) => {
                        failed += 1;
                        eprintln!("{session} failed: {error:#}");
                    }
                }
            }
            if failed > 0 {
                anyhow::bail!("{failed} sessions did not move; run move again");
            }
        }
        "move-orgs" => {
            let plane = args.get(1).context("a data plane id is required")?;
            anyhow::ensure!(
                db.dp_data_plane(plane).await?.is_some(),
                "data plane {plane} is not registered"
            );
            let archived = args.iter().any(|a| a == "--archived");
            let mut names: Vec<String> = args[2..]
                .iter()
                .filter(|a| a.as_str() != "--archived")
                .cloned()
                .collect();
            if names == ["-"] {
                let mut text = String::new();
                std::io::stdin().read_to_string(&mut text)?;
                names = text
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty() && !l.starts_with('#'))
                    .map(str::to_owned)
                    .collect();
            }
            anyhow::ensure!(!names.is_empty(), "name the orgs to move, or - for stdin");
            let mut orgs = Vec::with_capacity(names.len());
            for name in &names {
                orgs.push(org(&db, Some(name)).await?);
            }
            let drain = std::time::Duration::from_secs(120);
            let reports = data_plane::moves::move_orgs(&cp, plane, &orgs, archived, drain).await;
            let mut incomplete = 0;
            for report in &reports {
                let org = data_plane::org_public(report.org_id);
                match &report.error {
                    Some(error) => {
                        incomplete += 1;
                        eprintln!("{org} not moved: {error}");
                    }
                    None => {
                        if report.skipped + report.failed > 0 {
                            incomplete += 1;
                        }
                        println!(
                            "{org}{}: {} moved, {} skipped (in a turn), {} failed",
                            if report.enabled { " enabled" } else { "" },
                            report.moved,
                            report.skipped,
                            report.failed
                        );
                    }
                }
            }
            if incomplete > 0 {
                anyhow::bail!(
                    "{incomplete} of {} orgs are not fully moved; run move-orgs again",
                    reports.len()
                );
            }
        }
        "to-v1" => {
            let plane = args.get(1).context("a data plane id is required")?;
            let orgs = if args.len() > 2 {
                let mut orgs = Vec::new();
                for name in &args[2..] {
                    orgs.push(org(&db, Some(name)).await?);
                }
                orgs
            } else {
                db.dp_plane_orgs(plane).await?
            };
            let mut kept = 0;
            for org_id in orgs {
                let name = data_plane::org_public(org_id);
                match db.dp_return_org_to_v1(org_id).await? {
                    ReturnToV1::Returned(on) => {
                        db.dp_insert_org_event(
                            org_id,
                            "returned_to_v1",
                            &format!("left {on}"),
                            "operator",
                            chrono::Utc::now(),
                        )
                        .await?;
                        println!("{name} back on v1 (was on {on})");
                    }
                    ReturnToV1::HasSessions(n) => {
                        kept += 1;
                        println!("{name} kept on {plane}: {n} sessions run there");
                    }
                    ReturnToV1::NotOnDataPlane => println!("{name} already on v1"),
                }
            }
            if kept > 0 {
                anyhow::bail!("{kept} orgs have sessions on {plane} and stay there");
            }
        }
        other => {
            eprintln!("unknown command {other}\n{HELP}");
            std::process::exit(2);
        }
    }
    Ok(())
}
