//! Tool narration: backend-authored, human-readable lines for tool calls.
//!
//! Narration is contributed by the **capability that owns the tool**, via
//! [`crate::capabilities::Capability::narrate`]. everruns does not centrally
//! narrate tools by name — a capability (including host-registered plugins)
//! narrates its own tools, and unowned/foreign tools fall back to the generic
//! display-name phrasing here. See `specs/tool-narration.md`.
//!
//! This module provides:
//! - the [`ToolNarrationPhase`] enum and the generic fallback renderer, and
//! - reusable, locale-aware phrasing helpers (`narrate_read_file`,
//!   `narrate_shell_exec`, …) that capabilities call so wording and
//!   localization stay consistent without a global name registry.

use serde_json::Value;

use crate::localization::{
    BackendLocale, backend_strings, format_more_actions, localized_tool_display_name,
    resolve_backend_locale,
};
use crate::tool_types::{ToolCall, ToolDefinition};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolNarrationPhase {
    Started,
    Waiting,
    Completed,
    Failed,
}

fn title_case(name: &str) -> String {
    name.split(['_', ' '])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => {
                    let mut value = String::new();
                    value.extend(first.to_uppercase());
                    value.push_str(chars.as_str());
                    value
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn display_name(
    tool_def: Option<&ToolDefinition>,
    tool_call: &ToolCall,
    locale: Option<&str>,
) -> String {
    localized_tool_display_name(
        &tool_call.name,
        tool_def.and_then(|def| def.display_name()),
        locale,
    )
    .unwrap_or_else(|| title_case(&tool_call.name))
}

/// Read the first present, non-empty string argument among `keys`.
pub fn arg_str<'a>(arguments: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| arguments.get(*key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// Argument key fragments that may carry secrets. Narration never renders the
/// value of a field whose key contains one of these.
const SECRET_KEY_FRAGMENTS: &[&str] = &[
    "token",
    "api_key",
    "apikey",
    "password",
    "secret",
    "authorization",
];

fn is_secret_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    SECRET_KEY_FRAGMENTS
        .iter()
        .any(|fragment| lower.contains(fragment))
}

/// Like [`arg_str`] but never returns the value of a secret-bearing key, so
/// loosely-named arguments can't leak credentials into narration.
pub fn safe_arg_str<'a>(arguments: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .filter(|key| !is_secret_key(key))
        .find_map(|key| arguments.get(*key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// Final path component, for compact file narration.
pub fn basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    trimmed
        .rsplit('/')
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or(path)
}

/// Truncate a display value to `max_len` characters with an ellipsis.
pub fn truncate(value: &str, max_len: usize) -> String {
    let clean = value.trim();
    if clean.chars().count() <= max_len {
        return clean.to_string();
    }

    let truncated: String = clean.chars().take(max_len).collect();
    format!("{truncated}...")
}

/// Display form for a URL argument: host + path, with the scheme, `userinfo@`
/// credentials, query string, and fragment stripped (any of which can carry
/// secrets). Truncated.
pub fn url_display(url: &str) -> String {
    let without_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    // Drop query string and fragment.
    let host_path = without_scheme
        .split(['?', '#'])
        .next()
        .unwrap_or(without_scheme);
    // Split authority from path, then strip `user:pass@` userinfo from the
    // authority (it precedes the first '/').
    let (authority, path) = match host_path.split_once('/') {
        Some((authority, path)) => (authority, Some(path)),
        None => (host_path, None),
    };
    let host = authority
        .rsplit_once('@')
        .map(|(_, host)| host)
        .unwrap_or(authority);
    let rebuilt = match path {
        Some(path) => format!("{host}/{path}"),
        None => host.to_string(),
    };
    let cleaned = rebuilt.trim_end_matches('/');
    let cleaned = if cleaned.is_empty() {
        without_scheme
    } else {
        cleaned
    };
    truncate(cleaned, 48)
}

fn is_uk(locale: Option<&str>) -> bool {
    resolve_backend_locale(locale) == BackendLocale::Uk
}

type Verbs<'a> = (&'a str, &'a str, &'a str);

fn pick<'a>(locale: Option<&str>, en: Verbs<'a>, uk: Verbs<'a>) -> Verbs<'a> {
    if is_uk(locale) { uk } else { en }
}

/// `"{verb} {target}"`, or the bare verb when `target` is empty/absent.
pub fn generic_phrase(
    verb_started: &str,
    verb_completed: &str,
    verb_failed: &str,
    target: Option<String>,
    phase: ToolNarrationPhase,
) -> String {
    let verb = match phase {
        ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => verb_started,
        ToolNarrationPhase::Completed => verb_completed,
        ToolNarrationPhase::Failed => verb_failed,
    };

    match target {
        Some(target) if !target.is_empty() => format!("{verb} {target}"),
        _ => verb.to_string(),
    }
}

fn phrase3(verbs: Verbs, target: Option<String>, phase: ToolNarrationPhase) -> String {
    generic_phrase(verbs.0, verbs.1, verbs.2, target, phase)
}

/// `"{verb}: {value}"` when a value is present, otherwise the bare `"{verb}"`.
/// The neutral "Verb: argument" style ("Search tools: router").
pub fn labeled_phrase(
    verb_started: &str,
    verb_completed: &str,
    verb_failed: &str,
    value: Option<String>,
    phase: ToolNarrationPhase,
) -> String {
    let verb = match phase {
        ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => verb_started,
        ToolNarrationPhase::Completed => verb_completed,
        ToolNarrationPhase::Failed => verb_failed,
    };
    match value {
        Some(value) if !value.is_empty() => format!("{verb}: {value}"),
        _ => verb.to_string(),
    }
}

fn location_phrase(arguments: &Value, locale: Option<&str>) -> String {
    arg_str(arguments, &["path", "directory", "working_dir"])
        .map(|value| {
            if value == "." || value == "/workspace" {
                backend_strings(locale).current_directory.to_string()
            } else {
                value.to_string()
            }
        })
        .unwrap_or_else(|| backend_strings(locale).current_directory.to_string())
}

// ============================================================================
// Reusable phrasing helpers for capabilities
//
// Each helper owns the wording (English + Ukrainian where localized) for one
// tool family. Capabilities call these from their `narrate()` so wording and
// localization stay consistent without a global name registry.
// ============================================================================

/// Shell/exec command narration ("Ran `cargo test`"). `fallback` is used when
/// no command argument is present (typically the tool display name).
pub fn narrate_shell_exec(
    arguments: &Value,
    fallback: &str,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let command = arg_str(arguments, &["commands", "command"])
        .map(|value| format!("`{}`", truncate(value, 48)))
        .unwrap_or_else(|| fallback.to_string());
    let verbs = pick(
        locale,
        ("Running", "Ran", "Failed to run"),
        ("Запускаю", "Запустив", "Не вдалося запустити"),
    );
    phrase3(verbs, Some(command), phase)
}

fn path_target(arguments: &Value, keys: &[&str]) -> Option<String> {
    arg_str(arguments, keys).map(|path| basename(path).to_string())
}

/// `read_file` / `session_read_file` narration ("Read AGENTS.md").
pub fn narrate_read_file(
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let target = path_target(arguments, &["path"]);
    if is_uk(locale) {
        phrase3(
            ("Читаю", "Прочитав", "Не вдалося прочитати"),
            Some(target.unwrap_or_else(|| "файл".to_string())),
            phase,
        )
    } else {
        phrase3(("Reading", "Read", "Failed to read"), target, phase)
    }
}

/// `read_many_files` narration.
pub fn narrate_read_many_files(phase: ToolNarrationPhase, locale: Option<&str>) -> String {
    if is_uk(locale) {
        phrase3(
            (
                "Читаю кілька файлів",
                "Прочитав кілька файлів",
                "Не вдалося прочитати кілька файлів",
            ),
            None,
            phase,
        )
    } else {
        phrase3(
            ("Reading", "Read", "Failed to read"),
            Some("multiple files".to_string()),
            phase,
        )
    }
}

/// `write_file` narration.
pub fn narrate_write_file(
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let target = path_target(arguments, &["path"]);
    if is_uk(locale) {
        phrase3(
            ("Записую", "Записав", "Не вдалося записати"),
            Some(target.unwrap_or_else(|| "файл".to_string())),
            phase,
        )
    } else {
        phrase3(("Writing", "Wrote", "Failed to write"), target, phase)
    }
}

/// `edit_file` / `replace_in_file` narration.
pub fn narrate_edit_file(
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let target = path_target(arguments, &["path"]);
    if is_uk(locale) {
        phrase3(
            ("Редагую", "Відредагував", "Не вдалося відредагувати"),
            Some(target.unwrap_or_else(|| "файл".to_string())),
            phase,
        )
    } else {
        phrase3(("Editing", "Edited", "Failed to edit"), target, phase)
    }
}

/// `append_file` narration.
pub fn narrate_append_file(
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let target = path_target(arguments, &["path"]);
    if is_uk(locale) {
        phrase3(
            ("Дописую у", "Дописав у", "Не вдалося дописати у"),
            Some(target.unwrap_or_else(|| "файл".to_string())),
            phase,
        )
    } else {
        phrase3(
            ("Appending to", "Appended to", "Failed to append to"),
            target,
            phase,
        )
    }
}

/// `move_file` narration (reads destination keys before falling back to `path`).
pub fn narrate_move_file(
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let target = path_target(arguments, &["to", "destination", "new_path"])
        .or_else(|| path_target(arguments, &["path"]));
    if is_uk(locale) {
        phrase3(
            ("Переміщую", "Перемістив", "Не вдалося перемістити"),
            Some(target.unwrap_or_else(|| "файл".to_string())),
            phase,
        )
    } else {
        phrase3(("Moving", "Moved", "Failed to move"), target, phase)
    }
}

/// `delete_file` narration.
pub fn narrate_delete_file(
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let target = path_target(arguments, &["path"]);
    if is_uk(locale) {
        phrase3(
            ("Видаляю", "Видалив", "Не вдалося видалити"),
            Some(target.unwrap_or_else(|| "файл".to_string())),
            phase,
        )
    } else {
        phrase3(("Deleting", "Deleted", "Failed to delete"), target, phase)
    }
}

/// `mkdir` narration.
pub fn narrate_mkdir(arguments: &Value, phase: ToolNarrationPhase, locale: Option<&str>) -> String {
    let target = path_target(arguments, &["path"]);
    let verbs = pick(
        locale,
        (
            "Creating directory",
            "Created directory",
            "Failed to create directory",
        ),
        (
            "Створюю директорію",
            "Створив директорію",
            "Не вдалося створити директорію",
        ),
    );
    phrase3(verbs, target, phase)
}

/// `stat_file` narration.
pub fn narrate_stat_file(
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let target = path_target(arguments, &["path"]);
    if is_uk(locale) {
        phrase3(
            ("Перевіряю", "Перевірив", "Не вдалося перевірити"),
            Some(target.unwrap_or_else(|| "файл".to_string())),
            phase,
        )
    } else {
        phrase3(("Checking", "Checked", "Failed to check"), target, phase)
    }
}

/// `list_directory` / `list_files` narration.
pub fn narrate_list_directory(
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let target = location_phrase(arguments, locale);
    let verbs = pick(
        locale,
        (
            "Listing files in",
            "Listed files in",
            "Failed to list files in",
        ),
        (
            "Переглядаю файли у",
            "Переглянув файли у",
            "Не вдалося переглянути файли у",
        ),
    );
    phrase3(verbs, Some(target), phase)
}

/// `grep_files` narration.
pub fn narrate_grep_files(
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let pattern = arg_str(arguments, &["pattern"]).map(|pattern| truncate(pattern, 36));
    if is_uk(locale) {
        match pattern {
            Some(pattern) => phrase3(
                ("Шукаю", "Знайшов", "Не вдалося знайти"),
                Some(format!("`{pattern}` у файлах")),
                phase,
            ),
            None => phrase3(
                (
                    "Шукаю у файлах",
                    "Завершив пошук у файлах",
                    "Не вдалося виконати пошук у файлах",
                ),
                None,
                phase,
            ),
        }
    } else {
        let target = pattern
            .map(|pattern| format!("files for {pattern}"))
            .unwrap_or_else(|| "files".to_string());
        phrase3(
            ("Searching", "Searched", "Failed to search"),
            Some(target),
            phase,
        )
    }
}

/// Web-search narration ("Searched web for rust").
pub fn narrate_search_web(
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let query = arg_str(arguments, &["query", "q", "search"]).map(|query| truncate(query, 48));
    if is_uk(locale) {
        match query {
            Some(query) => labeled_phrase(
                "Шукаю у вебі",
                "Завершив пошук у вебі",
                "Не вдалося знайти у вебі",
                Some(query),
                phase,
            ),
            None => phrase3(
                (
                    "Шукаю у вебі",
                    "Завершив пошук у вебі",
                    "Не вдалося виконати пошук у вебі",
                ),
                None,
                phase,
            ),
        }
    } else {
        let target = query
            .map(|query| format!("web for {query}"))
            .unwrap_or_else(|| "web".to_string());
        phrase3(
            ("Searching", "Searched", "Failed to search"),
            Some(target),
            phase,
        )
    }
}

/// Generic provider/MCP search narration (e.g. tools ending in `__search`).
pub fn narrate_provider_search(
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let query = safe_arg_str(arguments, &["query", "q", "search", "pattern"])
        .map(|query| truncate(query, 48));
    if is_uk(locale) {
        labeled_phrase("Шукаю", "Завершив пошук", "Не вдалося знайти", query, phase)
    } else {
        labeled_phrase("Search", "Searched", "Could not search", query, phase)
    }
}

/// `secret_store` / `kv_store` operation narration.
pub fn narrate_secret_store(
    arguments: &Value,
    fallback: &str,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let operation = arg_str(arguments, &["operation"]).unwrap_or("use");
    let target = safe_arg_str(arguments, &["name", "key"])
        .map(str::to_string)
        .unwrap_or_else(|| fallback.to_string());
    if is_uk(locale) {
        match phase {
            ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => {
                format!("Виконую {} {}", title_case(operation), target)
                    .trim()
                    .to_string()
            }
            ToolNarrationPhase::Completed => {
                format!("Виконав {} {}", title_case(operation), target)
                    .trim()
                    .to_string()
            }
            ToolNarrationPhase::Failed => format!("Не вдалося виконати {operation} {target}")
                .trim()
                .to_string(),
        }
    } else {
        let started = format!("{}ing", title_case(operation));
        match phase {
            ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => {
                format!("{started} {target}").trim().to_string()
            }
            ToolNarrationPhase::Completed => {
                if operation.eq_ignore_ascii_case("list") {
                    format!("Listed {target}").trim().to_string()
                } else {
                    format!("{} {}", title_case(operation), target)
                        .trim()
                        .to_string()
                }
            }
            ToolNarrationPhase::Failed => {
                format!("Failed to {} {}", operation.to_lowercase(), target)
                    .trim()
                    .to_string()
            }
        }
    }
}

/// `spawn_subagent` narration.
pub fn narrate_spawn_subagent(
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let name = arg_str(arguments, &["name"]).map(|name| truncate(name, 40));
    if is_uk(locale) {
        let target = name
            .map(|name| format!("субагента {name}"))
            .unwrap_or_else(|| "субагента".to_string());
        phrase3(
            ("Запускаю", "Запустив", "Не вдалося запустити"),
            Some(target),
            phase,
        )
    } else {
        let target = name
            .map(|name| format!("{name} subagent"))
            .unwrap_or_else(|| "subagent".to_string());
        phrase3(
            ("Launching", "Launched", "Failed to launch"),
            Some(target),
            phase,
        )
    }
}

/// `write_todos` narration.
pub fn narrate_write_todos(phase: ToolNarrationPhase, locale: Option<&str>) -> String {
    let verbs = pick(
        locale,
        ("Updating", "Updated", "Failed to update"),
        (
            "Оновлюю список задач",
            "Оновив список задач",
            "Не вдалося оновити список задач",
        ),
    );
    if is_uk(locale) {
        phrase3(verbs, None, phase)
    } else {
        phrase3(verbs, Some("task list".to_string()), phase)
    }
}

/// `web_fetch` narration (English-only for now; UK falls back to English).
pub fn narrate_web_fetch(
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let value = safe_arg_str(arguments, &["url", "uri"]).map(url_display);
    let verbs = pick(
        locale,
        ("Fetch URL", "Fetched URL", "Could not fetch URL"),
        (
            "Завантажую URL",
            "Завантажив URL",
            "Не вдалося завантажити URL",
        ),
    );
    labeled_phrase(verbs.0, verbs.1, verbs.2, value, phase)
}

/// `tool_search` narration.
pub fn narrate_tool_search(
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let value = safe_arg_str(arguments, &["query"]).map(|q| truncate(q, 64));
    let verbs = pick(
        locale,
        ("Search tools", "Searched tools", "Could not search tools"),
        (
            "Шукаю інструменти",
            "Знайшов інструменти",
            "Не вдалося знайти інструменти",
        ),
    );
    labeled_phrase(verbs.0, verbs.1, verbs.2, value, phase)
}

/// Skill family narration: `activate_skill`, `read_skill`, `list_skills`.
pub fn narrate_skill(
    tool_name: &str,
    arguments: &Value,
    phase: ToolNarrationPhase,
    _locale: Option<&str>,
) -> Option<String> {
    let value = safe_arg_str(arguments, &["name", "skill", "id"]).map(|v| truncate(v, 48));
    let phrase = match tool_name {
        "activate_skill" => labeled_phrase(
            "Activate skill",
            "Activated skill",
            "Could not activate skill",
            value,
            phase,
        ),
        "read_skill" => labeled_phrase(
            "Read skill",
            "Read skill",
            "Could not read skill",
            value,
            phase,
        ),
        "list_skills" => generic_phrase(
            "Listing skills",
            "Listed skills",
            "Could not list skills",
            None,
            phase,
        ),
        _ => return None,
    };
    Some(phrase)
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
fn operation_narration(noun: &str, arguments: &Value, phase: ToolNarrationPhase) -> Option<String> {
    let operation = arg_str(arguments, &["operation", "action"])?;
    let verbs = operation_verbs(operation);
    let name =
        arg_str(arguments, &["display_name", "name", "title", "new_name"]).map(|v| truncate(v, 40));
    let target = match name {
        Some(name) => format!("{noun}: {name}"),
        None => noun.to_string(),
    };
    Some(phrase3(verbs, Some(target), phase))
}

/// Render the generic fallback narration for a tool call.
///
/// This no longer matches specific tool names — that is the job of the owning
/// capability's [`crate::capabilities::Capability::narrate`]. Here we only
/// apply the data-driven `narration_noun` operation narration and, failing
/// that, a `"{verb} {display_name}"` fallback.
pub fn render_tool_narration_with_locale(
    tool_def: Option<&ToolDefinition>,
    tool_call: &ToolCall,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let fallback_name = display_name(tool_def, tool_call, locale);

    if let Some(narration) = tool_def
        .and_then(|def| def.hints().narration_noun.as_deref())
        .and_then(|noun| operation_narration(noun, &tool_call.arguments, phase))
    {
        return narration;
    }

    let verbs = pick(
        locale,
        ("Running", "Ran", "Failed to run"),
        ("Запускаю", "Запустив", "Не вдалося запустити"),
    );
    phrase3(verbs, Some(fallback_name), phase)
}

pub fn render_tool_narration(
    tool_def: Option<&ToolDefinition>,
    tool_call: &ToolCall,
    phase: ToolNarrationPhase,
) -> String {
    render_tool_narration_with_locale(tool_def, tool_call, phase, None)
}

pub fn render_group_headline(
    tool_calls: &[ToolCall],
    tool_defs: &[ToolDefinition],
    phase: ToolNarrationPhase,
) -> Option<String> {
    render_group_headline_with_locale(tool_calls, tool_defs, phase, None)
}

pub fn render_group_headline_with_locale(
    tool_calls: &[ToolCall],
    tool_defs: &[ToolDefinition],
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> Option<String> {
    if tool_calls.is_empty() {
        return None;
    }

    let tool_map: std::collections::HashMap<&str, &ToolDefinition> =
        tool_defs.iter().map(|def| (def.name(), def)).collect();

    let phrases = tool_calls
        .iter()
        .map(|tool_call| {
            render_tool_narration_with_locale(
                tool_map.get(tool_call.name.as_str()).copied(),
                tool_call,
                phase,
                locale,
            )
        })
        .take(3)
        .collect::<Vec<_>>();

    Some(join_phrases(&phrases, tool_calls.len(), locale))
}

fn join_phrases(phrases: &[String], total_count: usize, locale: Option<&str>) -> String {
    let strings = backend_strings(locale);
    match phrases {
        [] => strings.working.to_string(),
        [only] => only.clone(),
        [first, second] => match resolve_backend_locale(locale) {
            BackendLocale::Uk => format!("{first} і {second}"),
            BackendLocale::En => format!("{first} and {second}"),
        },
        [first, second, ..] => {
            let more = format_more_actions(locale, total_count.saturating_sub(2));
            format!("{first}, {second}, {more}")
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::tool_types::ToolCall;

    fn args(value: serde_json::Value) -> serde_json::Value {
        value
    }

    #[test]
    fn shell_exec_helper_en_and_uk() {
        let a = args(json!({ "command": "cargo test" }));
        assert_eq!(
            narrate_shell_exec(&a, "Shell", ToolNarrationPhase::Started, None),
            "Running `cargo test`"
        );
        assert_eq!(
            narrate_shell_exec(&a, "Shell", ToolNarrationPhase::Completed, Some("uk")),
            "Запустив `cargo test`"
        );
    }

    #[test]
    fn read_file_helper_uses_basename() {
        let a = args(json!({ "path": "/workspace/AGENTS.md" }));
        assert_eq!(
            narrate_read_file(&a, ToolNarrationPhase::Started, None),
            "Reading AGENTS.md"
        );
        assert_eq!(
            narrate_read_file(&a, ToolNarrationPhase::Completed, Some("uk-UA")),
            "Прочитав AGENTS.md"
        );
    }

    #[test]
    fn edit_file_helper() {
        let a = args(json!({ "path": "/workspace/src/main.rs" }));
        assert_eq!(
            narrate_edit_file(&a, ToolNarrationPhase::Completed, None),
            "Edited main.rs"
        );
    }

    #[test]
    fn web_fetch_helper_strips_scheme_and_query() {
        let a = args(json!({ "url": "https://example.com/page?token=abc#frag" }));
        assert_eq!(
            narrate_web_fetch(&a, ToolNarrationPhase::Completed, None),
            "Fetched URL: example.com/page"
        );
        // Ukrainian locale must not fall back to English (no mixed-language UI).
        assert_eq!(
            narrate_web_fetch(&a, ToolNarrationPhase::Completed, Some("uk")),
            "Завантажив URL: example.com/page"
        );
    }

    #[test]
    fn url_display_strips_embedded_credentials() {
        assert_eq!(
            url_display("https://user:pass@example.com/path?token=abc"),
            "example.com/path"
        );
        assert_eq!(url_display("https://user:pass@example.com"), "example.com");
    }

    #[test]
    fn provider_search_never_leaks_secret() {
        let a = args(json!({ "token": "super-secret" }));
        assert_eq!(
            narrate_provider_search(&a, ToolNarrationPhase::Started, None),
            "Search"
        );
    }

    #[test]
    fn skill_helper_dispatches_family() {
        assert_eq!(
            narrate_skill(
                "activate_skill",
                &json!({ "name": "ship" }),
                ToolNarrationPhase::Completed,
                None
            ),
            Some("Activated skill: ship".to_string())
        );
        assert_eq!(
            narrate_skill("not_a_skill", &json!({}), ToolNarrationPhase::Started, None),
            None
        );
    }

    #[test]
    fn fallback_uses_narration_noun_when_present() {
        use crate::tool_types::{BuiltinTool, ToolDefinition, ToolHints};

        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "manage_agents".to_string(),
            arguments: json!({ "operation": "create", "name": "Neon Cartographer" }),
        };
        let def = ToolDefinition::Builtin(BuiltinTool {
            name: "manage_agents".to_string(),
            display_name: Some("Manage Agents".to_string()),
            description: String::new(),
            parameters: json!({}),
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints: ToolHints::default().with_narration_noun("agent"),
            full_parameters: None,
        });

        assert_eq!(
            render_tool_narration(Some(&def), &tool_call, ToolNarrationPhase::Started),
            "Creating agent: Neon Cartographer"
        );
        assert_eq!(
            render_tool_narration(Some(&def), &tool_call, ToolNarrationPhase::Completed),
            "Created agent: Neon Cartographer"
        );
        assert_eq!(
            render_tool_narration(Some(&def), &tool_call, ToolNarrationPhase::Failed),
            "Failed to create agent: Neon Cartographer"
        );
    }

    #[test]
    fn fallback_uses_display_name_for_unowned_tool() {
        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "mystery_tool".to_string(),
            arguments: json!({}),
        };
        assert_eq!(
            render_tool_narration(None, &tool_call, ToolNarrationPhase::Completed),
            "Ran Mystery Tool"
        );
    }
}
