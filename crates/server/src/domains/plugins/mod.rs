// Plugin domain — marketplace CRUD + sync + install lifecycle.
// See knowledge/integrations/plugins.md for the full specification.

use everruns_core::{Permission, Policy, Rule};

pub mod commands;
pub mod fetcher;
pub mod oauth_anchor;
pub mod queries;
pub mod types;

pub use commands::*;

/// View marketplaces and installed plugins.
pub const PLUGIN_VIEW: Policy = Policy {
    id: "plugin.view",
    rules: &[Rule::UserHasPermission(Permission::OrgPluginsView)],
};

/// Manage marketplaces and install/update/uninstall plugins.
/// Admin-gated per knowledge/integrations/plugins.md § Security.
pub const PLUGIN_MANAGE: Policy = Policy {
    id: "plugin.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgPluginsManage)],
};
