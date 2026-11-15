//! Portable trigger conditions.
use serde::{Deserialize, Serialize};
/// Conditions an incoming event must satisfy before a trigger runs.
///
/// Every condition must hold. A condition reads one dotted path of the event's
/// template context (`payload.action`, `event.type`, ...) and passes when the
/// value there equals any of `any_of`. Events that do not match are recorded as
/// `filtered` deliveries and start no session.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TriggerEventFilter {
    /// Conditions that must all hold.
    #[serde(default)]
    pub conditions: Vec<TriggerFilterCondition>,
    /// Pass only events whose value at a path differs from the last one seen
    /// for the same subject, such as a CI outcome flipping between `success`
    /// and `failure`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changes_only: Option<TriggerChangesOnly>,
}

/// "Only when it changes": the trigger remembers the last value at `path` per
/// event subject and filters out events that repeat it. The first event for a
/// subject only records its value, and an event with no value there (a
/// cancelled CI run has no outcome) is filtered without touching the record.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TriggerChangesOnly {
    /// Dotted path into the event context, e.g. `github.ci.outcome`.
    pub path: String,
}

/// One filter condition: the value at `path` must equal one of `any_of`.
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TriggerFilterCondition {
    /// Dotted path into the event context, e.g. `payload.action`.
    pub path: String,
    /// Accepted values. Strings compare exactly; other JSON values compare by
    /// equality.
    #[cfg_attr(feature="openapi",schema(value_type = Vec<Object>))]
    pub any_of: Vec<serde_json::Value>,
}
