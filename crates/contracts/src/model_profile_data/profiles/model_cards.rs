//! Vendor model cards and system cards, keyed by canonical model id.
//!
//! Kept apart from the profile payloads so a card link is one line to add
//! when a vendor publishes one, and so every profile route (direct provider,
//! gateway, key lookup) picks it up. Links come from the vendors' own
//! indexes: anthropic.com/system-cards, deploymentsafety.openai.com and Meta's
//! research blog. Several models share one card when the vendor published
//! them together.

const ANTHROPIC_OPUS_4_5: &str =
    "https://www-cdn.anthropic.com/bf10f64990cfda0ba858290be7b8cc6317685f47.pdf";
const ANTHROPIC_HAIKU_4_5: &str =
    "https://www-cdn.anthropic.com/7aad69bf12627d42234e01ee7c36305dc2f6a970.pdf";
const OPENAI_GPT_5_6: &str = "https://deploymentsafety.openai.com/gpt-5-6";
const OPENAI_GPT_6_OCTOBER: &str = "https://deploymentsafety.openai.com/gpt-6-october";

pub(super) fn model_card_url(canonical: &str) -> Option<&'static str> {
    // A 1M-context variant is the same model and shares its card.
    let id = canonical.strip_suffix("[1m]").unwrap_or(canonical);
    Some(match id {
        "claude-haiku-5-5" => "https://www.anthropic.com/claude-haiku-5-5-system-card",
        "claude-sonnet-5-5" => "https://www.anthropic.com/claude-sonnet-5-5-system-card",
        "claude-opus-5-5" => "https://www.anthropic.com/claude-opus-5-5-system-card",
        "claude-fable-5-1" => "https://www.anthropic.com/claude-fable-5-1-mythos-5-1-system-card",
        "claude-opus-5" => "https://www.anthropic.com/claude-opus-5-system-card",
        "claude-sonnet-5" => "https://www.anthropic.com/claude-sonnet-5-system-card",
        "claude-fable-5" => "https://www.anthropic.com/claude-fable-5-mythos-5-system-card",
        "claude-opus-4-8" => "https://www.anthropic.com/claude-opus-4-8-system-card",
        "claude-opus-4-7" => "https://www.anthropic.com/claude-opus-4-7-system-card",
        "claude-opus-4-6" => "https://www.anthropic.com/claude-opus-4-6-system-card",
        "claude-sonnet-4-6" => "https://www.anthropic.com/claude-sonnet-4-6-system-card",
        "claude-opus-4-5" => ANTHROPIC_OPUS_4_5,
        "claude-haiku-4-5" => ANTHROPIC_HAIKU_4_5,
        "gpt-6-astra" => "https://deploymentsafety.openai.com/gpt-6-astra",
        "gpt-6.1-sol" => "https://deploymentsafety.openai.com/gpt-6-1-sol",
        "gpt-6-sol" | "gpt-6-luna" => OPENAI_GPT_6_OCTOBER,
        "gpt-5.6-sol" | "gpt-5.6-terra" | "gpt-5.6-luna" => OPENAI_GPT_5_6,
        "muse-wonder-1.0" => "https://research.meta.ai/static/muse-wonder-model-card",
        _ => return None,
    })
}
