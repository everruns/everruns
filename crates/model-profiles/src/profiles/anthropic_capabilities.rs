use crate::types::{CLEAR_AT_PARAMETER, MID_CONVERSATION_SYSTEM_PARAMETER, ModelProfile};

pub(super) fn apply(profile: &mut ModelProfile) {
    profile.supports_server_compaction = matches!(
        profile.family.as_str(),
        "claude-fable-5-1"
            | "claude-fable-5"
            | "claude-opus-5"
            | "claude-opus-4-8"
            | "claude-opus-4-7"
            | "claude-opus-4-6"
            | "claude-sonnet-5"
            | "claude-sonnet-4-6"
    );
    if !matches!(
        profile.family.as_str(),
        "claude-fable-5-1"
            | "claude-fable-5"
            | "claude-opus-5-5"
            | "claude-opus-5"
            | "claude-opus-4-8"
            | "claude-sonnet-5-5"
    ) {
        return;
    }
    for parameter in [MID_CONVERSATION_SYSTEM_PARAMETER, CLEAR_AT_PARAMETER] {
        if !profile.supports_parameter(parameter) {
            profile.supported_parameters.push(parameter.to_string());
        }
    }
}

/// Whether a Claude model `family` supports Anthropic's hosted tool_search
/// (the `tool_search_tool_*_20251119` server tools). Per docs.claude.com, this
/// is Sonnet 4.0+, Opus 4.0+, Haiku 4.5+, and Fable 5.x — the 3.x families do not
/// support it. Centralized by family (rather than per-literal) because the rule is a
/// clean family cutoff; contrast the OpenAI profiles, which set `tool_search`
/// per model literal.
///
/// Only families with a corresponding profile in `anthropic_profile_data_inner`
/// belong here — Anthropic docs also list Mythos 5, but this registry has no
/// `claude-mythos-5` descriptor, so including it would be a dead branch. Add the
/// family here when (and if) its profile lands.
pub(super) fn supports_tool_search(family: &str) -> bool {
    matches!(
        family,
        "claude-fable-5-1"
            | "claude-fable-5"
            | "claude-opus-5-5"
            | "claude-opus-5"
            | "claude-opus-4-8"
            | "claude-opus-4-7"
            | "claude-opus-4-6"
            | "claude-opus-4-5"
            | "claude-opus-4"
            | "claude-sonnet-5-5"
            | "claude-sonnet-5"
            | "claude-sonnet-4-6"
            | "claude-haiku-4-5"
    )
}
