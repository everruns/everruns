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

#[cfg(test)]
mod tests {
    use super::*;

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
