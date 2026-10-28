//! Unified Knowledge: file catalog, immutable content and rebuildable indexes.
pub mod catalog;
pub mod commands;
pub mod content;
pub mod extraction;
pub mod files;
#[cfg(test)]
mod folder_tests;
pub mod indexing;
pub mod okf;
pub mod repository;
pub mod runtime;
pub mod sources;
#[cfg(test)]
mod tests;
pub mod types;
pub mod worker;
use crate::domains::common::{CommandError, Ctx};
use everruns_core::{Permission, Policy, Rule};
use std::sync::Arc;
use uuid::Uuid;
pub const VIEW: Policy = Policy {
    id: "knowledge.view",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsView)],
};
pub const MANAGE: Policy = Policy {
    id: "knowledge.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsManage)],
};

#[derive(Clone)]
pub struct KnowledgeService {
    pub db: Arc<crate::storage::StorageBackend>,
    pub encryption: Option<Arc<crate::storage::EncryptionService>>,
    pub drivers: Arc<everruns_contracts::driver_registry::DriverRegistry>,
    pub events: Option<Arc<crate::services::EventService>>,
    pub query_session: Option<everruns_contracts::typed_id::SessionId>,
    pub connections: Option<Arc<dyn everruns_core::connection_services::UserConnectionResolver>>,
}
impl KnowledgeService {
    pub fn from_ctx(ctx: &Ctx) -> Self {
        Self {
            db: ctx.db.clone(),
            events: ctx.event_service.clone(),
            query_session: None,
            encryption: ctx.encryption.clone(),
            drivers: ctx.driver_registry.clone(),
            connections: ctx.connection_resolver.clone(),
        }
    }
    pub fn files(db: Arc<crate::storage::StorageBackend>) -> Self {
        Self {
            db,
            events: None,
            query_session: None,
            encryption: None,
            drivers: Arc::new(Default::default()),
            connections: None,
        }
    }
}
pub fn id(value: &str, prefix: &str) -> Result<Uuid, CommandError> {
    let raw = value
        .strip_prefix(prefix)
        .ok_or_else(|| CommandError::not_found("Knowledge"))?;
    if raw.len() != 32 {
        return Err(CommandError::not_found("Knowledge"));
    }
    Uuid::parse_str(raw).map_err(|_| CommandError::not_found("Knowledge"))
}
pub fn public(id: Uuid, prefix: &str) -> String {
    format!("{prefix}{}", id.simple())
}
pub fn db_error(error: sqlx::Error) -> CommandError {
    if let sqlx::Error::Database(database) = &error {
        if database.is_unique_violation() {
            return CommandError::conflict("Knowledge resource already exists");
        }
        if database.is_foreign_key_violation() || database.is_check_violation() {
            return CommandError::bad_request(
                "Invalid Knowledge resource reference or configuration",
            );
        }
    }
    CommandError::internal(error.into())
}
pub fn path(value: &str) -> Result<String, CommandError> {
    if value.len() > 4096
        || value.contains(['\0', '\\'])
        || value.split('/').any(|v| v == ".." || v == ".")
    {
        return Err(CommandError::bad_request("Invalid knowledge path"));
    }
    Ok(format!(
        "/{}",
        value
            .trim_matches('/')
            .split('/')
            .filter(|p| !p.is_empty())
            .collect::<Vec<_>>()
            .join("/")
    ))
}
