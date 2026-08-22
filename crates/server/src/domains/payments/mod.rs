// Machine payments domain.

use everruns_core::{Permission, Policy, Rule};

pub mod authority;
pub mod commands;
pub mod eip712;
pub mod queries;
pub mod types;

pub use authority::ServerPaymentAuthority;
pub use commands::*;

pub const PAYMENT_MANAGE: Policy = Policy {
    id: "payment.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsManage)],
};

pub const PAYMENT_VIEW: Policy = Policy {
    id: "payment.view",
    rules: &[Rule::UserHasPermission(Permission::OrgSettingsView)],
};
