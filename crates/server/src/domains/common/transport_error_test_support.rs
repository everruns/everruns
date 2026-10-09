//! Test support for the command transport's error mapping.
//!
//! Extracted from `common.rs`, which is on the source-size ratchet: test
//! scaffolding does not belong in the production module. A child module of
//! `common`, like `error_tests.rs`.

use super::*;
use std::borrow::Cow;

pub(crate) const COMMAND_NAME: &str = "test_transport_conflict";
pub(crate) const RAW_DATABASE_DETAIL: &str = "duplicate key value violates unique constraint \"idx_transport_org_name\" at sqlx-postgres/src/connection.rs:666";
pub(crate) const SAFE_DOMAIN_DETAIL: &str = "Agent already exists";

#[derive(Debug)]
struct TestUniqueViolation;

impl std::fmt::Display for TestUniqueViolation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(RAW_DATABASE_DETAIL)
    }
}

impl std::error::Error for TestUniqueViolation {}

impl sqlx::error::DatabaseError for TestUniqueViolation {
    fn message(&self) -> &str {
        RAW_DATABASE_DETAIL
    }

    fn code(&self) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed("23505"))
    }

    fn as_error(&self) -> &(dyn std::error::Error + Send + Sync + 'static) {
        self
    }

    fn as_error_mut(&mut self) -> &mut (dyn std::error::Error + Send + Sync + 'static) {
        self
    }

    fn into_error(self: Box<Self>) -> Box<dyn std::error::Error + Send + Sync + 'static> {
        self
    }

    fn kind(&self) -> sqlx::error::ErrorKind {
        sqlx::error::ErrorKind::UniqueViolation
    }
}

#[derive(Debug, serde::Deserialize, utoipa::ToSchema, serde::Serialize)]
pub(crate) struct TransportConflictCommand {
    pub(crate) kind: String,
}

impl Command for TransportConflictCommand {
    type Output = serde_json::Value;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: COMMAND_NAME,
            category: "test",
            description: "Exercise command transport error handling.",
            method: "GET",
            path: "/test/transport-conflict",
        }
    }

    async fn execute(self, _ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let error = match self.kind.as_str() {
            "database" => anyhow::Error::new(sqlx::Error::database(TestUniqueViolation))
                .context("create resource"),
            "domain" => anyhow::anyhow!(SAFE_DOMAIN_DETAIL),
            other => anyhow::anyhow!("Unknown test conflict kind: {other}"),
        };
        Err(classify_anyhow(error))
    }
}

inventory::submit! {
    CommandDescriptor::of::<TransportConflictCommand>()
}
