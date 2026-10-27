//! `everruns-server --doctor`: the system doctor from the command line, with
//! no running server and no sign-in. It reads the same environment the server
//! would, so it answers "will this configuration work?" before a deploy, and
//! "what is wrong?" from inside a running container.
//!
//! Decisions:
//! - It runs every check the server runs, except that it is not one of the
//!   servers: the replica checks report the servers that heartbeat and never
//!   count this process.
//! - It never migrates and never writes, except the test object the storage
//!   check writes and deletes.
//! - Exit status 1 when any check is a problem, so a deploy script can gate
//!   on it; warnings do not fail it.

use super::{CheckStatus, DoctorEnv, DoctorReport, SystemDoctor};
use crate::auth::config::AuthConfig;
use crate::storage::{Database, EncryptionService, StorageBackend};
use anyhow::{Context, Result};
use everruns::utility_llm::SystemUtilityLlmConfig;
use everruns_durable::{PostgresWorkflowEventStore, WorkflowEventStore};
use std::sync::Arc;

/// Run the checks once against the services this environment names.
pub async fn run() -> Result<DoctorReport> {
    let database_url =
        std::env::var("DATABASE_URL").context("DATABASE_URL environment variable required")?;
    let database = Database::from_url(&database_url)
        .await
        .context("Failed to connect to the database in DATABASE_URL")?;
    let db = Arc::new(StorageBackend::from_database(database));
    let workflow_store: Arc<dyn WorkflowEventStore + Send + Sync> =
        Arc::new(PostgresWorkflowEventStore::new(db.pool().clone()));
    let encryption = EncryptionService::from_env().ok().map(Arc::new);
    let utility_llm = SystemUtilityLlmConfig::from_env()
        .into_service()
        .is_configured();
    let env = DoctorEnv::from_env(&AuthConfig::from_env(), utility_llm);
    let doctor = SystemDoctor::new(db, Some(workflow_store), encryption, env).preflight();
    Ok(doctor.run().await)
}

/// The report as plain text, problems first, for a terminal.
pub fn render(report: &DoctorReport) -> String {
    let mut out = format!(
        "Everruns {} system doctor: {}\n",
        report.version,
        status_word(report.status)
    );
    if !report.setup.is_empty() {
        out.push_str(&format!("Setup: {}\n", report.setup.join(", ")));
    }
    out.push('\n');
    for check in &report.checks {
        out.push_str(&format!(
            "{} {}: {}\n",
            status_mark(check.status),
            check.title,
            check.summary
        ));
        if check.status > CheckStatus::Info {
            if let Some(why) = &check.why {
                out.push_str(&format!("    Why: {why}\n"));
            }
            if let Some(fix) = &check.fix {
                out.push_str(&format!("    Fix: {fix}\n"));
            }
        }
    }
    out
}

/// The process exit status for a report.
pub fn exit_code(report: &DoctorReport) -> i32 {
    i32::from(report.status == CheckStatus::Problem)
}

fn status_word(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Ok | CheckStatus::Info => "ok",
        CheckStatus::Warning => "warnings",
        CheckStatus::Problem => "problems",
    }
}

fn status_mark(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Ok => "✓",
        CheckStatus::Info => "·",
        CheckStatus::Warning => "!",
        CheckStatus::Problem => "✗",
    }
}
