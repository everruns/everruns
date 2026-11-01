//! Rating: what one usage cost the org, and where that number came from.
//!
//! Order (knowledge/security/budgeting.md, "Cost of each usage"):
//! 1. host: the host reported what it charged (posted later, as a correction)
//! 2. rate: a matching row of the org's price list
//! 3. provider: the provider returned a cost with the response
//! 4. list_price: the model catalog estimate
//! 5. unpriced: no price anywhere. The cost is unknown, never zero, and
//!    never a token count dressed up as dollars.

use crate::storage::costs::CostRateRow;
use uuid::Uuid;

pub const METER_MODEL_TOKENS: &str = "model.tokens";
pub const METER_SANDBOX_TIME: &str = "sandbox.time";

/// Where a cost came from. Stored as `usage_ledger.basis`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostBasis {
    Host,
    Rate,
    Provider,
    ListPrice,
    Unpriced,
}

impl CostBasis {
    pub fn as_str(self) -> &'static str {
        match self {
            CostBasis::Host => "host",
            CostBasis::Rate => "rate",
            CostBasis::Provider => "provider",
            CostBasis::ListPrice => "list_price",
            CostBasis::Unpriced => "unpriced",
        }
    }
}

/// The cost of one usage.
#[derive(Debug, Clone, PartialEq)]
pub struct RatedCost {
    /// `None` when unpriced.
    pub amount: Option<f64>,
    pub basis: CostBasis,
    pub rate_id: Option<Uuid>,
}

impl RatedCost {
    fn unpriced() -> Self {
        RatedCost {
            amount: None,
            basis: CostBasis::Unpriced,
            rate_id: None,
        }
    }
}

/// Facts about one model call the rating needs.
#[derive(Debug, Clone, Default)]
pub struct ModelUsage<'a> {
    pub provider: Option<&'a str>,
    pub model: Option<&'a str>,
    /// Served by a host-managed provider (built-in models).
    pub managed: bool,
    pub total_tokens: i64,
    pub provider_cost_usd: Option<f64>,
    pub list_estimate_usd: Option<f64>,
}

/// Whether a rate's `match` covers this usage, and how specific it is
/// (number of keys). Unknown keys never match, so a typo cannot widen a rate.
fn match_score(rate: &CostRateRow, facts: &[(&str, Option<&str>)]) -> Option<usize> {
    let Some(map) = rate.match_.as_object() else {
        return Some(0);
    };
    let mut score = 0;
    for (key, expected) in map {
        let expected = expected.as_str()?;
        let actual = facts.iter().find(|(k, _)| k == key)?.1?;
        if !actual.eq_ignore_ascii_case(expected) {
            return None;
        }
        score += 1;
    }
    Some(score)
}

/// Most specific match first, then the latest start.
fn best_rate<'r>(
    rates: &'r [CostRateRow],
    facts: &[(&str, Option<&str>)],
) -> Option<&'r CostRateRow> {
    rates
        .iter()
        .filter_map(|rate| match_score(rate, facts).map(|score| (score, rate)))
        .max_by(|(sa, a), (sb, b)| sa.cmp(sb).then(a.effective_from.cmp(&b.effective_from)))
        .map(|(_, rate)| rate)
}

/// Rate a model call.
pub fn rate_model(usage: &ModelUsage<'_>, rates: &[CostRateRow]) -> RatedCost {
    let source = if usage.managed { "managed" } else { "own_keys" };
    let facts = [
        ("provider", usage.provider),
        ("model", usage.model),
        ("source", Some(source)),
    ];
    let reported = usage.provider_cost_usd.filter(|cost| *cost > 0.0);
    if let Some(rate) = best_rate(rates, &facts) {
        if let Some(price) = rate.unit_price_usd {
            // Model token rates are per million tokens, all buckets together.
            return RatedCost {
                amount: Some(usage.total_tokens.max(0) as f64 / 1_000_000.0 * price),
                basis: CostBasis::Rate,
                rate_id: Some(rate.id),
            };
        }
        if let Some(multiplier) = rate.list_price_multiplier {
            let base = reported.or(usage.list_estimate_usd);
            return match (base, multiplier) {
                (Some(base), _) => RatedCost {
                    amount: Some(base * multiplier),
                    basis: CostBasis::Rate,
                    rate_id: Some(rate.id),
                },
                // "Count usage, no cost" needs no base price.
                (None, 0.0) => RatedCost {
                    amount: Some(0.0),
                    basis: CostBasis::Rate,
                    rate_id: Some(rate.id),
                },
                (None, _) => RatedCost::unpriced(),
            };
        }
    }
    if let Some(cost) = reported {
        return RatedCost {
            amount: Some(cost),
            basis: CostBasis::Provider,
            rate_id: None,
        };
    }
    match usage.list_estimate_usd {
        Some(estimate) => RatedCost {
            amount: Some(estimate),
            basis: CostBasis::ListPrice,
            rate_id: None,
        },
        None => RatedCost::unpriced(),
    }
}

/// Rate sandbox running time. Sandboxes have no list price: without a rate
/// they are unpriced.
pub fn rate_sandbox(
    provider: &str,
    size: Option<&str>,
    seconds: f64,
    rates: &[CostRateRow],
) -> RatedCost {
    let facts = [("provider", Some(provider)), ("size", size)];
    match best_rate(rates, &facts) {
        Some(rate) => match rate.unit_price_usd {
            // Sandbox rates are per hour running.
            Some(price) => RatedCost {
                amount: Some(seconds.max(0.0) / 3600.0 * price),
                basis: CostBasis::Rate,
                rate_id: Some(rate.id),
            },
            None if rate.list_price_multiplier == Some(0.0) => RatedCost {
                amount: Some(0.0),
                basis: CostBasis::Rate,
                rate_id: Some(rate.id),
            },
            None => RatedCost::unpriced(),
        },
        None => RatedCost::unpriced(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};

    fn rate(matches: serde_json::Value, price: Option<f64>, mult: Option<f64>) -> CostRateRow {
        CostRateRow {
            id: Uuid::now_v7(),
            org_id: 1,
            meter: METER_MODEL_TOKENS.into(),
            match_: matches,
            unit_price_usd: price,
            list_price_multiplier: mult,
            unit: "1m_tokens".into(),
            effective_from: Utc::now() - Duration::days(1),
            retired: false,
            reason: None,
            created_by: None,
            created_at: Utc::now(),
        }
    }

    fn usage<'a>() -> ModelUsage<'a> {
        ModelUsage {
            provider: Some("openai"),
            model: Some("gpt-6.1-sol"),
            managed: false,
            total_tokens: 2_000_000,
            provider_cost_usd: None,
            list_estimate_usd: Some(4.0),
        }
    }

    #[test]
    fn falls_back_through_provider_list_and_unpriced() {
        let mut u = usage();
        assert_eq!(rate_model(&u, &[]).basis, CostBasis::ListPrice);
        u.provider_cost_usd = Some(3.0);
        let rated = rate_model(&u, &[]);
        assert_eq!(
            (rated.amount, rated.basis),
            (Some(3.0), CostBasis::Provider)
        );
        u.provider_cost_usd = None;
        u.list_estimate_usd = None;
        let rated = rate_model(&u, &[]);
        // An unknown price is unknown, never the token count as dollars.
        assert_eq!((rated.amount, rated.basis), (None, CostBasis::Unpriced));
    }

    #[test]
    fn multiplier_discounts_the_best_known_price() {
        let r = rate(serde_json::json!({"provider": "openai"}), None, Some(0.8));
        let rated = rate_model(&usage(), std::slice::from_ref(&r));
        assert_eq!(rated.basis, CostBasis::Rate);
        assert!((rated.amount.unwrap() - 3.2).abs() < 1e-9);
        assert_eq!(rated.rate_id, Some(r.id));
    }

    #[test]
    fn most_specific_match_wins_and_mismatch_never_applies() {
        let broad = rate(serde_json::json!({"provider": "openai"}), None, Some(0.8));
        let narrow = rate(
            serde_json::json!({"provider": "openai", "model": "gpt-6.1-sol"}),
            Some(1.0),
            None,
        );
        let other = rate(
            serde_json::json!({"provider": "anthropic"}),
            Some(99.0),
            None,
        );
        let rated = rate_model(&usage(), &[broad, narrow.clone(), other]);
        assert_eq!(rated.rate_id, Some(narrow.id));
        assert!((rated.amount.unwrap() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn source_filter_separates_built_in_models_from_own_keys() {
        let own = rate(serde_json::json!({"source": "own_keys"}), None, Some(0.5));
        let mut u = usage();
        u.managed = true;
        assert_eq!(rate_model(&u, &[own]).basis, CostBasis::ListPrice);
    }

    #[test]
    fn zero_multiplier_counts_usage_without_a_price() {
        let free = rate(serde_json::json!({"provider": "homelab"}), None, Some(0.0));
        let mut u = usage();
        u.provider = Some("homelab");
        u.list_estimate_usd = None;
        assert_eq!(rate_model(&u, &[free]).amount, Some(0.0));
    }

    #[test]
    fn sandboxes_are_unpriced_until_a_rate_exists() {
        assert_eq!(
            rate_sandbox("daytona", Some("small"), 7200.0, &[]).basis,
            CostBasis::Unpriced
        );
        let mut r = rate(serde_json::json!({"provider": "daytona"}), Some(0.06), None);
        r.meter = METER_SANDBOX_TIME.into();
        let rated = rate_sandbox("daytona", Some("small"), 7200.0, &[r]);
        assert!((rated.amount.unwrap() - 0.12).abs() < 1e-9);
    }
}
