use crate::types::{CLEAR_AT_PARAMETER, MID_CONVERSATION_SYSTEM_PARAMETER, ModelProfile};

pub(super) fn apply(profile: &mut ModelProfile) {
    if !matches!(
        profile.family.as_str(),
        "claude-fable-5-1"
            | "claude-fable-5"
            | "claude-opus-5-5"
            | "claude-opus-5"
            | "claude-opus-4-8"
    ) {
        return;
    }
    for parameter in [MID_CONVERSATION_SYSTEM_PARAMETER, CLEAR_AT_PARAMETER] {
        if !profile.supports_parameter(parameter) {
            profile.supported_parameters.push(parameter.to_string());
        }
    }
}
