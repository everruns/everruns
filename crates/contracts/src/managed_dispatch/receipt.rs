use super::{DispatchError, DispatchIdentity, DispatchPlan};
use serde::Deserialize;
use serde_json::value::RawValue;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptReview {
    IncompleteUsage,
    UnexpectedShape,
    ExceededBound,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageObservation {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_nanos: Option<u64>,
    /// Exact bounded supplier numeric lexeme; never round missing/invalid cost
    /// to zero. Retained for financial review, not as an accounting authority.
    pub cost_usd: Option<String>,
}

/// Only this protocol's host-observed response can construct a receipt. It is
/// neither deserializable nor an event-payload permission. The trusted host's
/// `record` implementation binds it to the originally admitted account/operation.
#[derive(Clone)]
pub struct SupplierReceipt {
    identity: DispatchIdentity,
    request_hash: [u8; 32],
    supplier_response_id: String,
    model: String,
    usage: UsageObservation,
    review: Option<ReceiptReview>,
}
impl SupplierReceipt {
    pub fn identity(&self) -> &DispatchIdentity {
        &self.identity
    }
    pub fn request_hash(&self) -> [u8; 32] {
        self.request_hash
    }
    pub fn supplier_response_id(&self) -> &str {
        &self.supplier_response_id
    }
    pub fn usage(&self) -> &UsageObservation {
        &self.usage
    }
    pub fn model(&self) -> &str {
        &self.model
    }
    pub fn review(&self) -> Option<ReceiptReview> {
        self.review
    }
}
impl std::fmt::Debug for SupplierReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SupplierReceipt")
            .field("usage", &self.usage)
            .field("review", &self.review)
            .finish_non_exhaustive()
    }
}

#[derive(Deserialize)]
struct WireUsage {
    prompt_tokens: Option<serde_json::Value>,
    completion_tokens: Option<serde_json::Value>,
    cost: Option<Box<RawValue>>,
    server_tool_use_details: Option<serde_json::Value>,
}
#[derive(Deserialize)]
struct WireResponse {
    id: String,
    model: Option<serde_json::Value>,
    openrouter_metadata: Option<serde_json::Value>,
    service_tier: Option<serde_json::Value>,
    usage: Option<Box<RawValue>>,
    choices: Option<Vec<serde_json::Value>>,
}

pub(super) fn parse(
    identity: DispatchIdentity,
    plan: &DispatchPlan,
    body: &[u8],
) -> Result<(SupplierReceipt, Option<String>), DispatchError> {
    let response: WireResponse =
        serde_json::from_slice(body).map_err(|_| DispatchError::Uncertain)?;
    if response.id.is_empty() || response.id.len() > 512 {
        return Err(DispatchError::Uncertain);
    }
    let wire_usage: Option<WireUsage> = response
        .usage
        .as_ref()
        .and_then(|raw| serde_json::from_str(raw.get()).ok());
    let usage = UsageObservation {
        input_tokens: wire_usage
            .as_ref()
            .and_then(|u| u.prompt_tokens.as_ref())
            .and_then(|n| n.as_u64()),
        output_tokens: wire_usage
            .as_ref()
            .and_then(|u| u.completion_tokens.as_ref())
            .and_then(|n| n.as_u64()),
        cost_nanos: wire_usage
            .as_ref()
            .and_then(|u| u.cost.as_ref())
            .and_then(|c| usd_nanos(c.get())),
        cost_usd: wire_usage
            .as_ref()
            .and_then(|u| u.cost.as_ref())
            .filter(|c| {
                c.get().len() <= 64
                    && c.get()
                        .bytes()
                        .all(|b| b.is_ascii_digit() || b"eE+-.".contains(&b))
            })
            .map(|c| c.get().to_string()),
    };
    let choice = response
        .choices
        .as_ref()
        .filter(|c| c.len() == 1)
        .and_then(|c| c.first());
    let content = choice
        .and_then(|c| c.pointer("/message/content"))
        .and_then(|c| c.as_str())
        .map(str::to_string);
    let no_tools = choice
        .and_then(|c| c.pointer("/message/tool_calls"))
        .is_none_or(|v| v.as_array().is_some_and(|tools| tools.is_empty()));
    let no_media = choice.and_then(|c| c.get("message")).is_some_and(|m| {
        ["audio", "images", "video", "function_call"]
            .iter()
            .all(|k| m.get(k).is_none())
    });
    let routing = response.openrouter_metadata.as_ref();
    let selected: Vec<_> = routing
        .and_then(|m| m.pointer("/endpoints/available"))
        .and_then(|v| v.as_array())
        .into_iter()
        .flatten()
        .filter(|v| v.get("selected").and_then(|s| s.as_bool()) == Some(true))
        .collect();
    let selected_matches = selected.len() == 1
        && selected[0].get("provider").and_then(|v| v.as_str()) == Some("OpenAI")
        && selected[0].get("model").and_then(|v| v.as_str()) == Some(&plan.model);
    let no_server_tools = wire_usage
        .as_ref()
        .and_then(|u| u.server_tool_use_details.as_ref())
        .is_none_or(|v| {
            ["tool_calls_requested", "tool_calls_executed"]
                .iter()
                .all(|k| v.get(k).and_then(|n| n.as_u64()) == Some(0))
        });
    let review = if response.model.as_ref().and_then(|m| m.as_str()) != Some(&plan.model)
        || routing
            .and_then(|m| m.get("attempt"))
            .and_then(|a| a.as_u64())
            != Some(1)
        || routing
            .and_then(|m| m.get("requested"))
            .and_then(|a| a.as_str())
            != Some(&plan.model)
        || routing
            .and_then(|m| m.get("is_byok"))
            .and_then(|a| a.as_bool())
            != Some(false)
        || response.service_tier.as_ref().and_then(|v| v.as_str()) != Some("default")
        || !selected_matches
        || !no_server_tools
        || content.is_none()
        || !no_tools
        || !no_media
    {
        Some(ReceiptReview::UnexpectedShape)
    } else if usage.input_tokens.is_none()
        || usage.output_tokens.is_none()
        || usage.cost_nanos.is_none()
    {
        Some(ReceiptReview::IncompleteUsage)
    } else if usage
        .input_tokens
        .is_some_and(|n| n > u64::from(plan.max_input_tokens))
        || usage
            .output_tokens
            .is_some_and(|n| n > u64::from(plan.max_output_tokens))
        || usage
            .cost_nanos
            .is_some_and(|n| n > plan.maximum_upstream_nanos())
    {
        Some(ReceiptReview::ExceededBound)
    } else {
        None
    };
    Ok((
        SupplierReceipt {
            identity,
            request_hash: plan.request_hash(),
            supplier_response_id: response.id,
            model: plan.model.clone(),
            usage,
            review,
        },
        content,
    ))
}

/// Parse positive USD JSON numbers into nanos with decimal ceiling, preserving
/// sub-nano costs and exponent notation without binary floating-point rounding.
fn usd_nanos(raw: &str) -> Option<u64> {
    if raw.len() > 64 || raw.starts_with('-') {
        return None;
    }
    let (base, exponent) = raw
        .split_once(['e', 'E'])
        .map_or(Some((raw, 0i32)), |(b, e)| Some((b, e.parse().ok()?)))?;
    let fraction = base.split_once('.').map_or(0, |(_, f)| f.len() as i32);
    let mantissa: u128 = base
        .chars()
        .filter(|c| *c != '.')
        .collect::<String>()
        .parse()
        .ok()?;
    if mantissa == 0 {
        return Some(0);
    }
    let power = exponent.checked_sub(fraction)?.checked_add(9)?;
    let nanos = if power >= 0 {
        mantissa.checked_mul(10u128.checked_pow(power as u32)?)?
    } else if power <= -39 {
        1
    } else {
        let divisor = 10u128.checked_pow((-power) as u32)?;
        mantissa / divisor + u128::from(!mantissa.is_multiple_of(divisor))
    };
    u64::try_from(nanos).ok().filter(|n| *n <= i64::MAX as u64)
}

#[cfg(test)]
mod tests {
    use super::usd_nanos;
    #[test]
    fn supplier_costs_keep_decimal_ceiling_and_do_not_invent_missing_or_overflowed_cost() {
        for (raw, expected) in [
            ("0", Some(0)),
            ("0.1", Some(100_000_000)),
            ("0.00000000001", Some(1)),
            ("1e-9", Some(1)),
            ("1e-99", Some(1)),
            ("1.0000000001", Some(1_000_000_001)),
            ("-1", None),
            ("null", None),
            ("1e30", None),
            ("9223372036.854775807", Some(i64::MAX as u64)),
            ("9223372036.854775808", None),
        ] {
            assert_eq!(usd_nanos(raw), expected, "{raw}");
        }
    }
}
