use crate::message::{Message, MessageRole};
use crate::tool_context::ReasoningEffortHandle;
use everruns_provider::DriverId;

pub(super) struct RequestControls {
    pub(super) reasoning_effort: Option<String>,
    pub(super) speed: Option<String>,
    pub(super) verbosity: Option<String>,
}

pub(super) fn resolve_request_controls(
    messages: &[Message],
    live_reasoning_effort: Option<&ReasoningEffortHandle>,
    provider_type: &DriverId,
    model: &str,
) -> RequestControls {
    let latest_user_controls = || {
        messages
            .iter()
            .rev()
            .find(|message| message.role == MessageRole::User)
            .and_then(|message| message.controls.as_ref())
    };

    let reasoning_effort = live_reasoning_effort
        .and_then(ReasoningEffortHandle::get)
        .or_else(|| {
            latest_user_controls()
                .and_then(|controls| controls.reasoning.as_ref())
                .and_then(|reasoning| reasoning.effort.clone())
        })
        .filter(|effort| {
            if effort.eq_ignore_ascii_case("none") {
                return false;
            }
            match crate::model_profiles::get_model_profile(provider_type, model) {
                Some(profile) if !profile.reasoning => {
                    tracing::warn!(
                        model,
                        effort,
                        "Stripping reasoning_effort: model does not support reasoning"
                    );
                    false
                }
                _ => true,
            }
        });

    let speed = latest_user_controls()
        .and_then(|controls| controls.speed.clone())
        .filter(|speed| {
            let Some(profile) = crate::model_profiles::get_model_profile(provider_type, model)
            else {
                return true;
            };
            let Some(speed_config) = profile.speed else {
                tracing::warn!(
                    model,
                    speed,
                    "Stripping speed: model does not support service tiers"
                );
                return false;
            };
            let supported = speed_config.values.iter().any(|value| {
                matches!(
                    (&value.value, speed.as_str()),
                    (crate::model::Speed::Flex, "flex")
                        | (crate::model::Speed::Default, "default")
                        | (crate::model::Speed::Priority, "priority")
                )
            });
            if !supported {
                tracing::warn!(
                    model,
                    speed,
                    "Stripping speed: model does not support requested service tier"
                );
            }
            supported
        });

    let verbosity = latest_user_controls()
        .and_then(|controls| controls.verbosity.clone())
        .filter(
            |verbosity| match crate::model_profiles::get_model_profile(provider_type, model) {
                Some(profile) if profile.verbosity.is_none() => {
                    tracing::warn!(
                        model,
                        verbosity,
                        "Stripping verbosity: model does not support verbosity control"
                    );
                    false
                }
                _ => true,
            },
        );

    RequestControls {
        reasoning_effort,
        speed,
        verbosity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_reasoning_effort_is_not_sent() {
        let mut message = Message::user("hello");
        message.controls = Some(crate::message::Controls {
            reasoning: Some(crate::message::ReasoningConfig {
                effort: Some("none".to_string()),
            }),
            ..Default::default()
        });

        let controls = resolve_request_controls(
            &[message],
            None,
            &DriverId::external("unknown"),
            "unknown-model",
        );
        assert!(controls.reasoning_effort.is_none());
    }
}
