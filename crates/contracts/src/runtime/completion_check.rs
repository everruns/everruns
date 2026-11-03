//! When the runtime checks that a finished turn did what was asked.
//!
//! Decision: the agent owns the setting, and the default is `unattended`. A
//! turn that ends on an announcement ("Submitting the review now") without
//! doing it is lost when nobody is watching: a trigger or schedule session has
//! no person to say "go on". In chat a person is there to notice, so the check
//! (one extra model call on the turns that need it) is opt-in with `always`.
//! See `knowledge/execution/turn-completion-check.md`.

use serde::{Deserialize, Serialize};

/// When a turn that is about to finish gets a completion check, and an
/// automatic continuation when the check finds the work unfinished.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum CompletionCheck {
    /// Never check.
    Off,
    /// Check sessions no person is watching: those started by triggers and
    /// schedules. Interactive chat is not checked.
    #[default]
    Unattended,
    /// Check every session, chat included.
    Always,
}

impl CompletionCheck {
    /// Whether this is the default, for `skip_serializing_if`.
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Unattended)
    }

    /// Whether a session checks its turns. `unattended` is the session's own
    /// fact: it was started by a trigger or schedule.
    pub fn applies(&self, unattended: bool) -> bool {
        match self {
            Self::Off => false,
            Self::Unattended => unattended,
            Self::Always => true,
        }
    }

    /// Wire and storage spelling.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Unattended => "unattended",
            Self::Always => "always",
        }
    }

    /// Parse the wire and storage spelling.
    pub fn from_str_opt(value: &str) -> Option<Self> {
        match value {
            "off" => Some(Self::Off),
            "unattended" => Some(Self::Unattended),
            "always" => Some(Self::Always),
            _ => None,
        }
    }

    /// Parse a stored or wire value. Absent or unknown reads as the default.
    pub fn from_wire(value: Option<&str>) -> Self {
        value.and_then(Self::from_str_opt).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spelling_round_trips_and_unknown_reads_as_default() {
        for check in [
            CompletionCheck::Off,
            CompletionCheck::Unattended,
            CompletionCheck::Always,
        ] {
            assert_eq!(CompletionCheck::from_str_opt(check.as_str()), Some(check));
            assert_eq!(
                serde_json::to_value(check).unwrap(),
                serde_json::json!(check.as_str())
            );
        }
        assert_eq!(
            CompletionCheck::from_wire(Some("sometimes")),
            CompletionCheck::Unattended
        );
        assert_eq!(
            CompletionCheck::from_wire(None),
            CompletionCheck::Unattended
        );
    }

    #[test]
    fn unattended_checks_only_sessions_nobody_watches() {
        assert!(!CompletionCheck::Off.applies(true));
        assert!(!CompletionCheck::Unattended.applies(false));
        assert!(CompletionCheck::Unattended.applies(true));
        assert!(CompletionCheck::Always.applies(false));
    }
}
