//! Live-endpoint exposure: which updates are a publication decision.
//!
//! Publishing and unpublishing take `AGENT_DANGEROUS`. An update that rewrites
//! a live endpoint's channel config (its auth, shared secrets, and public
//! surface) or turns it off has the same effect on who can reach the agent, so
//! it takes the same permission. Draft and disabled endpoints refuse traffic,
//! which keeps their edits under ordinary `AGENT_MANAGE` (EVE-1176).

use serde_json::Value;

use crate::domains::agents::AGENT_DANGEROUS;
use crate::domains::common::{CommandError, Ctx};

/// THREAT[TM-AUTHZ-022]: a manage-only member must not weaken, rotate, or
/// disable a live endpoint without the owner-only publication gate.
pub(super) fn require_live_change_permission(
    ctx: &Ctx,
    endpoint_status: &str,
    config_changed: bool,
    disabling: bool,
) -> Result<(), CommandError> {
    if endpoint_status != "live" || !(config_changed || disabling) {
        return Ok(());
    }
    AGENT_DANGEROUS
        .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
        .map_err(|e| CommandError::forbidden(e.message))
}

/// Whether `next` (normalized, secrets merged) differs from the stored config.
///
/// Both sides are canonicalized so re-saving what a read returned is not a
/// change: the redaction-only `*_configured` flags are dropped and inline auth
/// is round-tripped through its typed form. Anything that still differs,
/// including a stored config that no longer validates (`None`), counts as a
/// change, so the comparison fails closed.
pub(super) fn config_changed(next: &Value, current: Option<Value>) -> bool {
    current.is_none_or(|current| canonical(next.clone()) != canonical(current))
}

fn canonical(mut config: Value) -> Value {
    strip_configured_flags(&mut config);
    if let Some(map) = config.as_object_mut() {
        match map.get("auth") {
            Some(Value::Null) => {
                map.remove("auth");
            }
            Some(auth) => {
                if let Some(typed) =
                    serde_json::from_value::<crate::records::EndpointAuthConfig>(auth.clone())
                        .ok()
                        .and_then(|auth| serde_json::to_value(auth).ok())
                {
                    map.insert("auth".to_string(), typed);
                }
            }
            None => {}
        }
    }
    config
}

fn strip_configured_flags(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.retain(|key, value| !(key.ends_with("_configured") && value.is_boolean()));
            map.values_mut().for_each(strip_configured_flags);
        }
        Value::Array(items) => items.iter_mut().for_each(strip_configured_flags),
        _ => {}
    }
}
