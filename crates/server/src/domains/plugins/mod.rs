// Plugin domain — marketplace CRUD + sync + install lifecycle.
// See knowledge/integrations/plugins.md for the full specification.

use everruns_core::{Permission, Policy, Rule};

pub mod choices;
pub mod commands;
pub mod fetcher;
mod lookup;
pub mod oauth_anchor;
mod patch;
pub mod queries;
pub mod session_load;
pub mod types;

pub use choices::{ListMyPlugins, SetMyPluginChoice};
pub use commands::*;
pub use patch::*;
pub use session_load::WorkerPluginsCall;

/// View marketplaces and installed plugins.
pub const PLUGIN_VIEW: Policy = Policy {
    id: "plugin.view",
    rules: &[Rule::UserHasPermission(Permission::OrgPluginsView)],
};

/// Manage marketplaces and install/update/uninstall plugins.
/// Requires admin permissions.
pub const PLUGIN_MANAGE: Policy = Policy {
    id: "plugin.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgPluginsManage)],
};
