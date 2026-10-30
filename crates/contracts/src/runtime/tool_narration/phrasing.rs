use super::{ToolNarrationPhase, arg_str, generic_phrase, is_uk, labeled_phrase, truncate};
use crate::runtime::localization::runtime_text;
use serde_json::Value;

type Verbs<'a> = (&'a str, &'a str, &'a str);

pub(super) fn pick<'a>(locale: Option<&str>, en: Verbs<'a>, uk: Verbs<'a>) -> Verbs<'a> {
    if is_uk(locale) {
        uk
    } else {
        (
            runtime_text(locale, en.0),
            runtime_text(locale, en.1),
            runtime_text(locale, en.2),
        )
    }
}

pub(super) fn phrase3(
    locale: Option<&str>,
    verbs: Verbs,
    target: Option<String>,
    phase: ToolNarrationPhase,
) -> String {
    generic_phrase(
        runtime_text(locale, verbs.0),
        runtime_text(locale, verbs.1),
        runtime_text(locale, verbs.2),
        target,
        phase,
    )
}

pub(super) fn localized_labeled_phrase(
    locale: Option<&str>,
    started: &str,
    completed: &str,
    failed: &str,
    value: Option<String>,
    phase: ToolNarrationPhase,
) -> String {
    labeled_phrase(
        runtime_text(locale, started),
        runtime_text(locale, completed),
        runtime_text(locale, failed),
        value,
        phase,
    )
}

/// Map a CRUD operation to (started, completed, failed) verb forms.
fn operation_verbs(operation: &str) -> Verbs<'static> {
    match operation {
        "create" => ("Creating", "Created", "Failed to create"),
        "update" => ("Updating", "Updated", "Failed to update"),
        "delete" | "destroy" => ("Deleting", "Deleted", "Failed to delete"),
        "copy" | "clone" | "duplicate" => ("Copying", "Copied", "Failed to copy"),
        "list" => ("Listing", "Listed", "Failed to list"),
        "get" | "read" => ("Reading", "Read", "Failed to read"),
        "set" => ("Setting", "Set", "Failed to set"),
        "send" => ("Sending", "Sent", "Failed to send"),
        "run" | "execute" => ("Running", "Ran", "Failed to run"),
        _ => ("Running", "Ran", "Failed to run"),
    }
}

/// Build narration from `narration_noun` + `operation` arg.
/// E.g. noun="agent", operation="create", name="Neon Cartographer"
/// → "Creating agent: Neon Cartographer".
pub(super) fn operation_narration(
    locale: Option<&str>,
    noun: &str,
    arguments: &Value,
    phase: ToolNarrationPhase,
) -> Option<String> {
    let operation = arg_str(arguments, &["operation", "action"])?;
    let verbs = operation_verbs(operation);
    let noun = runtime_text(locale, noun);
    let name =
        arg_str(arguments, &["display_name", "name", "title", "new_name"]).map(|v| truncate(v, 40));
    let target = match name {
        Some(name) => format!("{noun}: {name}"),
        None => noun.to_string(),
    };
    Some(phrase3(locale, verbs, Some(target), phase))
}
