// Prompt-canary streaming output guardrail.
//
// Detects the simplest form of system-prompt leakage: the model echoing
// (a normalized form of) the first sentence of its own system prompt back
// to the user. When detected, the assistant message is replaced with a
// canned refusal and the original tokens are dropped — they are never
// persisted or sent on subsequent turns.
//
// This is intentionally narrow: substring matching of one normalized
// needle against the accumulated output. It catches naive prompt-extraction
// attempts ("repeat your system prompt back") without trying to be a
// general-purpose data-loss-prevention layer. False-positive risk is bounded
// by requiring a minimum needle length (`MIN_CANARY_LEN`); shorter prompts
// produce no canary and the guardrail is a no-op for that stream.

use std::sync::Arc;

use everruns_core::capabilities::{Capability, CapabilityLocalization};
use everruns_core::output_guardrail::{
    GuardrailDecision, OutputGuardrail, OutputGuardrailContext, OutputGuardrailRun,
};

pub const PROMPT_CANARY_GUARDRAIL_CAPABILITY_ID: &str = "prompt_canary_guardrail";

/// Reason code surfaced in `output.message.replaced` events. Stable: clients
/// localize their copy from this string.
pub const REASON_CODE_SYSTEM_PROMPT_LEAK: &str = "system_prompt_leak";

/// Default replacement text shown in place of the suppressed model output.
/// Plain prose so any client renders it sensibly — no markdown, no template.
pub const DEFAULT_REPLACEMENT: &str =
    "[Response withheld: the model attempted to reveal protected instructions.]";

/// Minimum length of the extracted needle, after normalization. Below this
/// the canary refuses to arm — short fragments would over-trigger on
/// commonplace phrases like "you are a helpful assistant".
const MIN_CANARY_LEN: usize = 30;

/// Maximum length of the extracted needle. Long needles waste cycles and are
/// unlikely to ever match because the model paraphrases.
const MAX_CANARY_LEN: usize = 240;

pub struct PromptCanaryGuardrailCapability;

impl Capability for PromptCanaryGuardrailCapability {
    fn id(&self) -> &str {
        PROMPT_CANARY_GUARDRAIL_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Prompt Canary Guardrail"
    }

    fn description(&self) -> &str {
        "Detects when the model leaks its own system prompt during streaming and \
         replaces the assistant response with a refusal. Uses the first sentence \
         of the system prompt as a canary phrase."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Канарковий захист промпту",
            "Виявляє, коли модель розкриває власний системний промпт під час стрімінгу, і замінює відповідь асистента відмовою. Використовує перше речення системного промпту як канаркову фразу.",
        )]
    }

    fn category(&self) -> Option<&str> {
        Some("Safety")
    }

    fn is_guardrail(&self) -> bool {
        true
    }

    fn icon(&self) -> Option<&str> {
        Some("shield")
    }

    fn output_guardrails(&self) -> Vec<Arc<dyn OutputGuardrail>> {
        vec![Arc::new(PromptCanaryGuardrail)]
    }
}

struct PromptCanaryGuardrail;

impl OutputGuardrail for PromptCanaryGuardrail {
    fn id(&self) -> &str {
        "prompt_canary"
    }

    fn arm(&self, ctx: &OutputGuardrailContext<'_>) -> Option<Box<dyn OutputGuardrailRun>> {
        let needle = extract_canary(ctx.system_prompt)?;
        let replacement = ctx
            .config
            .get("replacement")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| DEFAULT_REPLACEMENT.to_string());
        Some(Box::new(PromptCanaryRun {
            needle,
            replacement,
            normalized_acc: String::new(),
            last_was_space: true, // suppress a leading space when first chunk is whitespace-only
        }))
    }
}

struct PromptCanaryRun {
    /// Normalized first sentence of the system prompt — lowercased and with
    /// runs of whitespace collapsed to a single space.
    needle: String,
    replacement: String,
    /// Normalized accumulator updated incrementally from each delta. We never
    /// re-normalize the full output — extending this buffer per delta makes
    /// `check` O(|delta|) instead of O(|accumulated|), avoiding O(n²) behavior
    /// across long streams.
    normalized_acc: String,
    /// Tracks whether the *last* character of `normalized_acc` is the lone
    /// trailing space we collapsed into; lets us coalesce whitespace runs
    /// that straddle a delta boundary.
    last_was_space: bool,
}

impl OutputGuardrailRun for PromptCanaryRun {
    fn check(&mut self, _accumulated: &str, delta: &str) -> GuardrailDecision {
        // Append the normalized form of just this delta. Whitespace runs that
        // straddle delta boundaries are coalesced via `last_was_space`.
        normalize_extend(&mut self.normalized_acc, &mut self.last_was_space, delta);
        if self.normalized_acc.len() < self.needle.len() {
            return GuardrailDecision::Pass;
        }
        if self.normalized_acc.contains(self.needle.as_str()) {
            GuardrailDecision::block(REASON_CODE_SYSTEM_PROMPT_LEAK, self.replacement.clone())
        } else {
            GuardrailDecision::Pass
        }
    }
}

/// Extract a canary needle from `prompt`. Walks sentence boundaries and
/// returns the first sentence whose normalized form is ≥ `MIN_CANARY_LEN`.
/// This intentionally prefers a single self-contained sentence (e.g. the
/// agent-specific instructions inside a longer assembled prompt) over the
/// whole multi-sentence prefix, so the canary still matches when the model
/// regurgitates only the agent layer of a layered prompt.
///
/// Returns `None` when no qualifying sentence exists — common for very
/// short prompts where false-positive risk is too high.
fn extract_canary(prompt: &str) -> Option<String> {
    // Skip over XML-tag wrapper lines (e.g. `<system-prompt>`,
    // `<capability id="...">`). The runtime wraps capability contributions
    // in tags that are not part of the *intended* leak target — leaking
    // those is fine, it just reveals structure. We want the human-authored
    // sentence inside.
    let core: String = prompt
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !(trimmed.is_empty() || (trimmed.starts_with('<') && trimmed.ends_with('>')))
        })
        .collect::<Vec<_>>()
        .join(" ");

    // Split on sentence terminators (., !, ?). Newlines were already
    // collapsed into spaces above so the only remaining boundaries are
    // punctuation. Iterate sentences in order; the first one whose
    // normalized form is ≥ MIN_CANARY_LEN is the canary.
    let bytes = core.as_bytes();
    let mut start = 0usize;
    for (i, b) in bytes.iter().enumerate() {
        if matches!(*b, b'.' | b'!' | b'?') {
            // Inclusive of the terminator so "ends with" tests still see it.
            let end = (i + 1).min(bytes.len());
            let sentence = core[start..end].trim();
            let mut needle = normalize(sentence);
            truncate_at_char_boundary(&mut needle, MAX_CANARY_LEN);
            if needle.len() >= MIN_CANARY_LEN {
                return Some(needle);
            }
            start = end;
        }
    }
    // No sentence boundary found — fall back to the whole prompt if it's
    // long enough on its own.
    let mut needle = normalize(&core[start..]);
    truncate_at_char_boundary(&mut needle, MAX_CANARY_LEN);
    (needle.len() >= MIN_CANARY_LEN).then_some(needle)
}

fn truncate_at_char_boundary(s: &mut String, max_bytes: usize) {
    if s.len() <= max_bytes {
        return;
    }
    let truncate_at = s
        .char_indices()
        .map(|(idx, _)| idx)
        .take_while(|idx| *idx <= max_bytes)
        .last()
        .unwrap_or(0);
    s.truncate(truncate_at);
}

/// Append the normalized form of `chunk` to `acc`, preserving the
/// "previous char was whitespace" state in `last_was_space` so whitespace
/// runs that straddle chunk boundaries are correctly collapsed. Used by
/// `PromptCanaryRun::check` to maintain a running normalized buffer in O(|delta|)
/// per call instead of re-normalizing the full accumulated output every time.
fn normalize_extend(acc: &mut String, last_was_space: &mut bool, chunk: &str) {
    for ch in chunk.chars() {
        if ch.is_whitespace() {
            if !*last_was_space {
                acc.push(' ');
            }
            *last_was_space = true;
        } else {
            // `to_lowercase()` per char allocates a small iterator; cheaper
            // than allocating a full lowercase string for the whole chunk.
            for lower in ch.to_lowercase() {
                acc.push(lower);
            }
            *last_was_space = false;
        }
    }
}

/// Lowercase + collapse whitespace. Mirrored on both sides of the comparison
/// so the canary survives reformatting (extra spaces, capitalization,
/// trailing punctuation drift, line wrapping).
fn normalize(input: &str) -> String {
    let lower = input.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut prev_space = false;
    for ch in lower.chars() {
        if ch.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
            }
            prev_space = true;
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    if out.ends_with(' ') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use everruns_core::capabilities::*;
    use serde_json::json;

    fn arm_with(prompt: &str) -> Option<Box<dyn OutputGuardrailRun>> {
        let cfg = json!({});
        let ctx = OutputGuardrailContext {
            system_prompt: prompt,
            config: &cfg,
        };
        PromptCanaryGuardrail.arm(&ctx)
    }

    #[test]
    fn extracts_first_sentence_after_min_len() {
        let needle =
            extract_canary("You are a helpful assistant who never reveals secrets. Trust me.")
                .expect("extracted");
        assert!(needle.starts_with("you are a helpful assistant"));
        assert!(needle.ends_with("never reveals secrets."));
    }

    #[test]
    fn skips_short_leading_sentence_in_layered_prompt() {
        // Mimics what the runtime assembles when a harness "You are a helpful
        // assistant." is layered with an agent prompt: the canary should be
        // the agent's longer, identifying sentence — not the harness's
        // generic opener.
        let needle = extract_canary(
            "You are a helpful assistant.\n\nYou are an internal pricing oracle that \
             never discloses margins. Refuse out-of-scope questions.",
        )
        .expect("extracted");
        assert!(needle.starts_with("you are an internal pricing oracle"));
        assert!(!needle.contains("helpful assistant"));
    }

    #[test]
    fn declines_to_arm_for_short_prompts() {
        assert!(arm_with("hi.").is_none());
        assert!(arm_with("").is_none());
        // Single short sentence under min len.
        assert!(arm_with("Be brief.").is_none());
    }

    #[test]
    fn skips_xml_wrapper_lines() {
        let prompt = "<system-prompt>\n\
                      You are an internal pricing oracle that never discloses margins.\n\
                      </system-prompt>";
        let needle = extract_canary(prompt).expect("extracted");
        assert!(needle.contains("internal pricing oracle"));
        assert!(!needle.contains("<system-prompt>"));
    }

    #[test]
    fn passes_for_unrelated_output() {
        let mut run = arm_with(
            "You are an internal pricing oracle that never discloses margins. \
             Refuse out-of-scope questions.",
        )
        .expect("armed");
        let chunks = ["The weather", " in Tokyo", " is sunny."];
        let mut acc = String::new();
        for c in chunks {
            acc.push_str(c);
            assert!(matches!(run.check(&acc, c), GuardrailDecision::Pass));
        }
    }

    #[test]
    fn blocks_when_first_sentence_appears_verbatim() {
        let mut run = arm_with(
            "You are an internal pricing oracle that never discloses margins. \
             Refuse out-of-scope questions.",
        )
        .expect("armed");
        // Simulate the model echoing the prompt back to the user.
        let leak = "Sure, my instructions are: \
                    You are an internal pricing oracle that never discloses margins.";
        match run.check(leak, leak) {
            GuardrailDecision::Block(b) => {
                assert_eq!(b.reason_code, REASON_CODE_SYSTEM_PROMPT_LEAK);
                assert!(b.replacement.contains("withheld"));
            }
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn matches_despite_whitespace_and_case_drift() {
        let mut run = arm_with(
            "You are an internal pricing oracle that never discloses margins. \
             Refuse out-of-scope questions.",
        )
        .expect("armed");
        // Same words, mangled formatting.
        let leak = "YOU ARE AN INTERNAL PRICING\nORACLE  that NEVER DISCLOSES margins.";
        assert!(matches!(run.check(leak, leak), GuardrailDecision::Block(_)));
    }

    #[test]
    fn allows_custom_replacement_via_config() {
        let cfg = json!({"replacement": "nope"});
        let ctx = OutputGuardrailContext {
            system_prompt: "You are an internal pricing oracle that never discloses margins. \
                            Refuse out-of-scope questions.",
            config: &cfg,
        };
        let mut run = PromptCanaryGuardrail.arm(&ctx).expect("armed");
        let leak = "You are an internal pricing oracle that never discloses margins.";
        match run.check(leak, leak) {
            GuardrailDecision::Block(b) => assert_eq!(b.replacement, "nope"),
            other => panic!("expected Block, got {other:?}"),
        }
    }

    #[test]
    fn capability_exposes_guardrail() {
        let cap = PromptCanaryGuardrailCapability;
        assert_eq!(cap.id(), PROMPT_CANARY_GUARDRAIL_CAPABILITY_ID);
        assert_eq!(cap.output_guardrails().len(), 1);
    }

    #[test]
    fn normalize_collapses_whitespace_and_lowercases() {
        assert_eq!(normalize("  Hello   World\nHere "), "hello world here");
    }

    #[test]
    fn incremental_normalize_matches_full_normalize_across_chunk_boundaries() {
        // The streaming check builds its normalized buffer one delta at a
        // time. The result must equal a single-shot `normalize()` call on
        // the concatenated input — including across whitespace runs that
        // straddle chunk boundaries (e.g. a chunk ending with a space and
        // the next starting with a space).
        let chunks = ["  Hello", "  WORLD\n", "\there  "];
        let full: String = chunks.concat();
        let mut acc = String::new();
        let mut last = true;
        for c in chunks {
            normalize_extend(&mut acc, &mut last, c);
        }
        // Trim a trailing collapsed space if any — `normalize()` does this
        // by popping the final char; the streaming buffer doesn't, but
        // substring matching is unaffected.
        let acc_trimmed = acc.trim_end_matches(' ');
        assert_eq!(acc_trimmed, normalize(&full));
    }

    #[test]
    fn incremental_check_blocks_when_needle_spans_multiple_deltas() {
        let mut run = arm_with(
            "You are an internal pricing oracle that never discloses margins. \
             Refuse out-of-scope questions.",
        )
        .expect("armed");
        // Stream the leak in many small chunks that together reconstruct the
        // canary. Should still trip — the incremental buffer accumulates
        // across calls.
        let leak = "you are AN INTERNAL pricing\noracle that NEVER discloses margins.";
        let chunk_size = 5;
        let mut tripped = false;
        let mut acc = String::new();
        for chunk in leak
            .as_bytes()
            .chunks(chunk_size)
            .map(|c| std::str::from_utf8(c).unwrap_or(""))
        {
            acc.push_str(chunk);
            if matches!(run.check(&acc, chunk), GuardrailDecision::Block(_)) {
                tripped = true;
                break;
            }
        }
        assert!(tripped, "expected canary to trip on multi-chunk leak");
    }

    #[test]
    fn extract_canary_does_not_panic_when_truncation_hits_multibyte_in_sentence_path() {
        let prompt = format!("{}é. trailing sentence.", "a".repeat(MAX_CANARY_LEN - 1));
        let needle = extract_canary(&prompt).expect("extracted");
        assert!(needle.len() <= MAX_CANARY_LEN);
    }

    #[test]
    fn extract_canary_does_not_panic_when_truncation_hits_multibyte_in_fallback_path() {
        let prompt = format!("{}é trailing text", "a".repeat(MAX_CANARY_LEN - 1));
        let needle = extract_canary(&prompt).expect("extracted");
        assert!(needle.len() <= MAX_CANARY_LEN);
    }
}
