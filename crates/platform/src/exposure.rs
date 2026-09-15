use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

pub const DEFAULT_PUBLIC_TOOL_ACTIVITY_TEXT: &str = "Working...";

/// How much tool activity a public surface may expose.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum PublicToolVisibility {
    /// Do not expose tool activity.
    None,
    /// Expose only editable generic text, without tool names, args, or output.
    #[default]
    Generic,
    /// Expose backend-authored narration, without raw tool names, args, or output.
    Narrated,
}

/// The only tool-activity text a public surface may show, or `None` when tool
/// activity must not be exposed at all.
///
/// `Narrated` deliberately resolves to the same generic text as `Generic`:
/// backend- or model-authored narration can derive from raw tool-call arguments,
/// so it is not safe to forward. The empty-value fallback exists because the text
/// is user-editable and an empty status is worse than a generic one.
pub fn public_tool_activity_text(
    visibility: PublicToolVisibility,
    generic_tool_text: &str,
) -> Option<&str> {
    match visibility {
        PublicToolVisibility::None => None,
        PublicToolVisibility::Generic | PublicToolVisibility::Narrated => {
            let trimmed = generic_tool_text.trim();
            Some(if trimmed.is_empty() {
                DEFAULT_PUBLIC_TOOL_ACTIVITY_TEXT
            } else {
                trimmed
            })
        }
    }
}

/// Conversation starters a surface should offer for a fresh thread.
///
/// The agent's starters win whenever it has any; otherwise the (already
/// inheritance-folded) harness starters apply. This is the same precedence
/// Platform Chat renders with — see `Agent::starters` and
/// `check_platform_chat_content` — lifted here so every exposure resolves it
/// identically instead of each transport re-deciding.
///
/// Empty on both sides means the surface has nothing authored for it. Callers
/// must render nothing rather than substituting generic prompts: a prompt
/// nobody wrote is worse than an empty pane.
pub fn resolve_starters<'a>(
    agent_starters: &'a [crate::ConversationStarter],
    harness_starters: &'a [crate::ConversationStarter],
) -> &'a [crate::ConversationStarter] {
    if agent_starters.is_empty() {
        harness_starters
    } else {
        agent_starters
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starter(text: &str) -> crate::ConversationStarter {
        crate::ConversationStarter {
            icon: None,
            text: text.to_string(),
        }
    }

    #[test]
    fn resolve_starters_prefers_the_agent_then_falls_back_to_the_harness() {
        let agent = vec![starter("Triage the newest P1")];
        let harness = vec![starter("Summarize this channel")];

        assert_eq!(
            resolve_starters(&agent, &harness),
            agent.as_slice(),
            "an agent with starters must win over the harness"
        );
        assert_eq!(
            resolve_starters(&[], &harness),
            harness.as_slice(),
            "an agent with no starters must inherit the harness ones"
        );
        // Both empty means nothing was authored. Callers render nothing; a
        // generic prompt nobody wrote is worse than an empty pane.
        assert!(
            resolve_starters(&[], &[]).is_empty(),
            "nothing authored must stay nothing"
        );
    }

    #[test]
    fn public_tool_activity_text_is_the_one_policy() {
        assert_eq!(
            public_tool_activity_text(PublicToolVisibility::None, "Reading the payroll table"),
            None,
            "None must expose nothing, including a configured string"
        );

        // Narrated is not a licence to forward narration: it can derive from raw
        // tool-call arguments, so it resolves to the same safe text as Generic.
        for visibility in [
            PublicToolVisibility::Generic,
            PublicToolVisibility::Narrated,
        ] {
            assert_eq!(
                public_tool_activity_text(visibility, "Looking that up"),
                Some("Looking that up"),
                "{visibility:?}"
            );
            assert_eq!(
                public_tool_activity_text(visibility, "  Looking that up  "),
                Some("Looking that up"),
                "{visibility:?} must trim"
            );
            // The text is user-editable, and an empty status is worse than a
            // generic one.
            assert_eq!(
                public_tool_activity_text(visibility, "   "),
                Some(DEFAULT_PUBLIC_TOOL_ACTIVITY_TEXT),
                "{visibility:?} must fall back when the configured text is blank"
            );
        }
    }
}
