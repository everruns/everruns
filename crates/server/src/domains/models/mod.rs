// LLM models domain — commands, queries, types.
//
// See knowledge/foundations/domains.md for the pattern.

use everruns_core::{Permission, Policy, Rule};

pub mod catalog;
pub mod commands;
pub mod queries;
pub mod record;
pub mod service;
pub mod sync;
pub mod types;

pub use commands::*;
pub use service::*;
pub use sync::{ModelSyncService, SyncResult};

pub const LLM_MODEL_VIEW: Policy = Policy {
    id: "model.view",
    rules: &[Rule::UserHasPermission(Permission::OrgModelsView)],
};
pub const LLM_MODEL_MANAGE: Policy = Policy {
    id: "model.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgModelsManage)],
};
