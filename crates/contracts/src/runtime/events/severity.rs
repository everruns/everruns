//! Shared vocabulary for how bad a failure is.

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// How bad a recorded failure is, decided by whether work stopped.
///
/// The runtime assigns this, not the client: a failed tool call is reported
/// back to the model as a result and the turn keeps going, so it is an
/// `issue`. A failure that ends the work (a failed turn, a failed task) is an
/// `error`. Clients render issues as warnings and reserve
/// error styling for `error`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum FailureSeverity {
    /// Recoverable: the failure was handed back and the work carried on.
    Issue,
    /// Fatal: the work stopped because of the failure.
    Error,
}
