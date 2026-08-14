//! Tool narration: backend-authored, human-readable lines for tool calls.
//!
//! Narration is contributed by the **capability that owns the tool**, via
//! [`crate::capabilities::Capability::narrate`]. everruns does not centrally
//! narrate tools by name — a capability (including host-registered plugins)
//! narrates its own tools, and unowned/foreign tools fall back to the generic
//! display-name phrasing here. See `knowledge/execution/tool-narration.md`.
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
use crate::session_files::SessionFileSystem;
use crate::session_path::WORKSPACE_PREFIX;
use crate::tool_types::{ToolCall, ToolDefinition};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolNarrationPhase {
    Started,
    Waiting,
    Completed,
    Failed,
}

/// Optional execution context for path-bearing tool narration.
///
/// When a [`SessionFileSystem`] is present, path-bearing helpers resolve input
/// through its path contract so narration matches tool results.
#[derive(Clone, Copy, Default)]
pub struct ToolNarrationContext<'a> {
    pub file_store: Option<&'a dyn SessionFileSystem>,
}

impl<'a> ToolNarrationContext<'a> {
    pub fn new(file_store: Option<&'a dyn SessionFileSystem>) -> Self {
        Self { file_store }
    }
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
    let trimmed = url.trim();
    let parseable = trimmed
        .strip_prefix("//")
        .map(|rest| format!("https://{rest}"))
        .unwrap_or_else(|| trimmed.to_string());
    let Ok(parsed) = url::Url::parse(&parseable) else {
        return String::new();
    };

    let Some(host) = parsed.host().map(|host| host.to_string()) else {
        return String::new();
    };

    let authority = match parsed.port() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    };
    let cleaned_path = parsed.path().trim_end_matches('/');
    let display = if cleaned_path.is_empty() {
        authority
    } else {
        format!("{authority}{cleaned_path}")
    };

    let cleaned = display.trim_end_matches('/');
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

/// Present a filesystem path for narration using the active store contract.
///
/// When `file_store` is absent (generic fallback / offline builders), falls
/// back to the legacy argument echo with localized root phrasing.
pub fn present_filesystem_path(
    file_store: Option<&dyn SessionFileSystem>,
    input: Option<&str>,
    locale: Option<&str>,
) -> String {
    if let Some(store) = file_store {
        let raw = input
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(WORKSPACE_PREFIX);
        return if store.is_mount_resolver() {
            store.resolve_path(raw)
        } else {
            store.display_path(raw)
        };
    }
    location_phrase_from_raw(input, locale)
}

fn location_phrase(
    arguments: &Value,
    locale: Option<&str>,
    ctx: ToolNarrationContext<'_>,
) -> String {
    let raw = arg_str(arguments, &["path", "directory", "working_dir"]);
    present_filesystem_path(ctx.file_store, raw, locale)
}

fn location_phrase_from_raw(input: Option<&str>, locale: Option<&str>) -> String {
    input
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            if value == "." || value == WORKSPACE_PREFIX {
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
    ctx: ToolNarrationContext<'_>,
) -> String {
    let target = location_phrase(arguments, locale, ctx);
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
        labeled_phrase("Searching", "Searched", "Could not search", query, phase)
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
        let verbs = match operation.to_ascii_lowercase().as_str() {
            "get" | "read" => ("Getting", "Got", "Failed to get"),
            "set" => ("Setting", "Set", "Failed to set"),
            "delete" => ("Deleting", "Deleted", "Failed to delete"),
            "list" => ("Listing", "Listed", "Failed to list"),
            _ => ("Using", "Used", "Failed to use"),
        };
        phrase3(verbs, Some(target), phase)
    }
}

/// Subagent delegation narration.
pub fn narrate_subagent_spawn(
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

/// `web_fetch` narration, distinguishing inline fetches from file downloads.
pub fn narrate_web_fetch(
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let value = safe_arg_str(arguments, &["url", "uri"]).map(url_display);
    let is_download = arguments
        .get("save_to_file")
        .and_then(Value::as_str)
        .is_some_and(|path| !path.trim().is_empty());
    let verbs = if is_download {
        pick(
            locale,
            (
                "Downloading URL",
                "Downloaded URL",
                "Could not download URL",
            ),
            (
                "Завантажую URL",
                "Завантажив URL",
                "Не вдалося завантажити URL",
            ),
        )
    } else {
        pick(
            locale,
            ("Fetching URL", "Fetched URL", "Could not fetch URL"),
            ("Отримую URL", "Отримав URL", "Не вдалося отримати URL"),
        )
    };
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
        (
            "Searching tools",
            "Searched tools",
            "Could not search tools",
        ),
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
            "Activating skill",
            "Activated skill",
            "Could not activate skill",
            value,
            phase,
        ),
        "read_skill" => labeled_phrase(
            "Reading skill",
            "Read skill",
            "Could not read skill",
            value,
            phase,
        ),
        "write_skill" => labeled_phrase(
            "Writing skill",
            "Wrote skill",
            "Could not write skill",
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

/// `write_session_title` narration. The proposed title is shown as bounded
/// detail (truncated), never an unbounded echo.
pub fn narrate_write_session_title(
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let title = safe_arg_str(arguments, &["title"]).map(|title| truncate(title, 48));
    let verbs = pick(
        locale,
        (
            "Updating session title",
            "Updated session title",
            "Failed to update session title",
        ),
        (
            "Оновлюю назву сесії",
            "Оновив назву сесії",
            "Не вдалося оновити назву сесії",
        ),
    );
    labeled_phrase(verbs.0, verbs.1, verbs.2, title, phase)
}

/// `get_session_info` narration.
pub fn narrate_get_session_info(phase: ToolNarrationPhase, locale: Option<&str>) -> String {
    let verbs = pick(
        locale,
        (
            "Reading session info",
            "Read session info",
            "Failed to read session info",
        ),
        (
            "Читаю інформацію про сесію",
            "Прочитав інформацію про сесію",
            "Не вдалося прочитати інформацію про сесію",
        ),
    );
    phrase3(verbs, None, phase)
}

/// Session-schedule family narration: `create_schedule`, `cancel_schedule`,
/// `list_schedules`. Returns `None` for names outside the family.
pub fn narrate_session_schedule(
    tool_name: &str,
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> Option<String> {
    let phrase = match tool_name {
        "create_schedule" => {
            let detail = safe_arg_str(arguments, &["description"]).map(|value| truncate(value, 48));
            let verbs = pick(
                locale,
                (
                    "Creating schedule",
                    "Created schedule",
                    "Failed to create schedule",
                ),
                (
                    "Створюю розклад",
                    "Створив розклад",
                    "Не вдалося створити розклад",
                ),
            );
            labeled_phrase(verbs.0, verbs.1, verbs.2, detail, phase)
        }
        "cancel_schedule" => {
            let verbs = pick(
                locale,
                (
                    "Cancelling schedule",
                    "Cancelled schedule",
                    "Failed to cancel schedule",
                ),
                (
                    "Скасовую розклад",
                    "Скасував розклад",
                    "Не вдалося скасувати розклад",
                ),
            );
            phrase3(verbs, None, phase)
        }
        "list_schedules" => {
            let verbs = pick(
                locale,
                (
                    "Listing schedules",
                    "Listed schedules",
                    "Failed to list schedules",
                ),
                (
                    "Переглядаю розклади",
                    "Переглянув розклади",
                    "Не вдалося переглянути розклади",
                ),
            );
            phrase3(verbs, None, phase)
        }
        _ => return None,
    };
    Some(phrase)
}

/// Session-task family narration: `list_tasks`, `get_task`, `message_task`,
/// `cancel_task`, `wait_task`. Returns `None` for names outside the family.
pub fn narrate_session_task(
    tool_name: &str,
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> Option<String> {
    // `task_id` is a stable, non-secret identifier; show it as bounded detail.
    let task = safe_arg_str(arguments, &["task_id"]).map(|id| truncate(id, 32));
    let phrase = match tool_name {
        "list_tasks" => {
            let verbs = pick(
                locale,
                ("Listing tasks", "Listed tasks", "Failed to list tasks"),
                (
                    "Переглядаю завдання",
                    "Переглянув завдання",
                    "Не вдалося переглянути завдання",
                ),
            );
            phrase3(verbs, None, phase)
        }
        "get_task" => {
            let verbs = pick(
                locale,
                ("Reading task", "Read task", "Could not read task"),
                (
                    "Читаю завдання",
                    "Прочитав завдання",
                    "Не вдалося прочитати завдання",
                ),
            );
            labeled_phrase(verbs.0, verbs.1, verbs.2, task, phase)
        }
        "message_task" => {
            let verbs = pick(
                locale,
                ("Messaging task", "Messaged task", "Could not message task"),
                (
                    "Надсилаю повідомлення завданню",
                    "Надіслав повідомлення завданню",
                    "Не вдалося надіслати повідомлення завданню",
                ),
            );
            labeled_phrase(verbs.0, verbs.1, verbs.2, task, phase)
        }
        "cancel_task" => {
            let verbs = pick(
                locale,
                ("Cancelling task", "Cancelled task", "Could not cancel task"),
                (
                    "Скасовую завдання",
                    "Скасував завдання",
                    "Не вдалося скасувати завдання",
                ),
            );
            labeled_phrase(verbs.0, verbs.1, verbs.2, task, phase)
        }
        "wait_task" => {
            let verbs = pick(
                locale,
                (
                    "Waiting for task",
                    "Task finished",
                    "Could not wait for task",
                ),
                (
                    "Очікую на завдання",
                    "Завдання завершено",
                    "Не вдалося дочекатися завдання",
                ),
            );
            labeled_phrase(verbs.0, verbs.1, verbs.2, task, phase)
        }
        _ => return None,
    };
    Some(phrase)
}

/// `sandbox_status` narration.
pub fn narrate_sandbox_status(phase: ToolNarrationPhase, locale: Option<&str>) -> String {
    let verbs = pick(
        locale,
        (
            "Checking sandbox status",
            "Checked sandbox status",
            "Failed to check sandbox status",
        ),
        (
            "Перевіряю стан пісочниці",
            "Перевірив стан пісочниці",
            "Не вдалося перевірити стан пісочниці",
        ),
    );
    phrase3(verbs, None, phase)
}

/// `sandbox_manage` narration; the lifecycle `action` (pause/resume/delete) is
/// shown as bounded detail.
pub fn narrate_sandbox_manage(
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let action = arg_str(arguments, &["action"]).map(|action| truncate(action, 24));
    let verbs = pick(
        locale,
        (
            "Managing sandbox",
            "Managed sandbox",
            "Failed to manage sandbox",
        ),
        (
            "Керую пісочницею",
            "Виконав дію над пісочницею",
            "Не вдалося виконати дію над пісочницею",
        ),
    );
    labeled_phrase(verbs.0, verbs.1, verbs.2, action, phase)
}

/// Session SQL family narration: `sql_execute`, `sql_query`, `sql_schema`.
/// SQL text is intentionally omitted because statements can contain sensitive values.
pub fn narrate_sql(
    tool_name: &str,
    _arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> Option<String> {
    let phrase = match tool_name {
        "sql_execute" => {
            let verbs = pick(
                locale,
                ("Running SQL", "Ran SQL", "Failed to run SQL"),
                ("Виконую SQL", "Виконав SQL", "Не вдалося виконати SQL"),
            );
            phrase3(verbs, None, phase)
        }
        "sql_query" => {
            let verbs = pick(
                locale,
                ("Querying SQL", "Queried SQL", "Failed to query SQL"),
                (
                    "Запитую SQL",
                    "Виконав SQL-запит",
                    "Не вдалося виконати SQL-запит",
                ),
            );
            phrase3(verbs, None, phase)
        }
        "sql_schema" => {
            let verbs = pick(
                locale,
                (
                    "Reading database schema",
                    "Read database schema",
                    "Failed to read database schema",
                ),
                (
                    "Читаю схему бази даних",
                    "Прочитав схему бази даних",
                    "Не вдалося прочитати схему бази даних",
                ),
            );
            phrase3(verbs, None, phase)
        }
        _ => return None,
    };
    Some(phrase)
}

/// `search_knowledge` / `search_index` narration ("Searched knowledge: orders").
pub fn narrate_search_knowledge(
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let query = safe_arg_str(arguments, &["query", "q", "search"]).map(|query| truncate(query, 48));
    let verbs = pick(
        locale,
        (
            "Searching knowledge",
            "Searched knowledge",
            "Could not search knowledge",
        ),
        (
            "Шукаю у знаннях",
            "Знайшов у знаннях",
            "Не вдалося знайти у знаннях",
        ),
    );
    labeled_phrase(verbs.0, verbs.1, verbs.2, query, phase)
}

/// `get_current_time` narration.
pub fn narrate_current_time(phase: ToolNarrationPhase, locale: Option<&str>) -> String {
    let verbs = pick(
        locale,
        (
            "Checking current time",
            "Checked current time",
            "Failed to check current time",
        ),
        (
            "Перевіряю поточний час",
            "Перевірив поточний час",
            "Не вдалося перевірити поточний час",
        ),
    );
    phrase3(verbs, None, phase)
}

/// `check_budget` narration.
pub fn narrate_check_budget(phase: ToolNarrationPhase, locale: Option<&str>) -> String {
    let verbs = pick(
        locale,
        (
            "Checking budget",
            "Checked budget",
            "Failed to check budget",
        ),
        (
            "Перевіряю бюджет",
            "Перевірив бюджет",
            "Не вдалося перевірити бюджет",
        ),
    );
    phrase3(verbs, None, phase)
}

/// `query_history` narration ("Searched history: deploy").
pub fn narrate_query_history(
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let query = safe_arg_str(arguments, &["query", "q", "search"]).map(|query| truncate(query, 48));
    let verbs = pick(
        locale,
        (
            "Searching history",
            "Searched history",
            "Could not search history",
        ),
        (
            "Шукаю в історії",
            "Знайшов в історії",
            "Не вдалося знайти в історії",
        ),
    );
    labeled_phrase(verbs.0, verbs.1, verbs.2, query, phase)
}

/// `spawn_background` narration; the target tool name (a safe, bounded
/// identifier) is shown as detail.
pub fn narrate_spawn_background(
    arguments: &Value,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let target = safe_arg_str(arguments, &["tool"]).map(|tool| truncate(tool, 40));
    let verbs = pick(
        locale,
        (
            "Starting background run",
            "Started background run",
            "Failed to start background run",
        ),
        (
            "Запускаю фоновий процес",
            "Запустив фоновий процес",
            "Не вдалося запустити фоновий процес",
        ),
    );
    labeled_phrase(verbs.0, verbs.1, verbs.2, target, phase)
}

/// Delegation-result family narration: `report_result`, `report_task_progress`.
pub fn narrate_delegation_result(
    tool_name: &str,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> Option<String> {
    let verbs = match tool_name {
        "report_result" => pick(
            locale,
            (
                "Reporting result",
                "Reported result",
                "Failed to report result",
            ),
            (
                "Повідомляю результат",
                "Повідомив результат",
                "Не вдалося повідомити результат",
            ),
        ),
        "report_task_progress" => pick(
            locale,
            (
                "Reporting progress",
                "Reported progress",
                "Failed to report progress",
            ),
            (
                "Повідомляю про прогрес",
                "Повідомив про прогрес",
                "Не вдалося повідомити про прогрес",
            ),
        ),
        _ => return None,
    };
    Some(phrase3(verbs, None, phase))
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
    if let [tool_call] = tool_calls {
        return Some(render_tool_narration_with_locale(
            tool_map.get(tool_call.name.as_str()).copied(),
            tool_call,
            phase,
            locale,
        ));
    }

    let actions = tool_calls
        .iter()
        .map(|tool_call| {
            let tool_def = tool_map.get(tool_call.name.as_str()).copied();
            let repeated_narration = render_tool_narration_with_locale(
                tool_def,
                &tool_call_for_group_summary(tool_call),
                phase,
                locale,
            );
            GroupHeadlineAction::new(
                tool_call,
                render_tool_narration_with_locale(tool_def, tool_call, phase, locale),
                repeated_narration,
            )
        })
        .collect::<Vec<_>>();

    Some(summarize_group_actions(&actions, locale))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GroupHeadlineAction {
    tool_name: String,
    narration: String,
    repeated_narration: String,
}

impl GroupHeadlineAction {
    pub(crate) fn new(tool_call: &ToolCall, narration: String, repeated_narration: String) -> Self {
        Self {
            tool_name: tool_call.name.clone(),
            narration,
            repeated_narration,
        }
    }
}

/// Retain only arguments that distinguish operation types within a tool family.
pub(crate) fn tool_call_for_group_summary(tool_call: &ToolCall) -> ToolCall {
    let mut arguments = serde_json::Map::new();
    for key in ["operation", "action"] {
        if let Some(value) = tool_call.arguments.get(key) {
            arguments.insert(key.to_string(), value.clone());
        }
    }
    ToolCall {
        id: tool_call.id.clone(),
        name: tool_call.name.clone(),
        arguments: Value::Object(arguments),
    }
}

/// Collapse equivalent actions and bound a batch headline to two distinct summaries.
pub(crate) fn summarize_group_actions(
    actions: &[GroupHeadlineAction],
    locale: Option<&str>,
) -> String {
    let strings = backend_strings(locale);
    if actions.is_empty() {
        return strings.working.to_string();
    }
    if let [only] = actions {
        return only.narration.clone();
    }

    let mut grouped: Vec<(&GroupHeadlineAction, usize)> = Vec::new();
    let mut indexes = std::collections::HashMap::<(&str, &str), usize>::new();
    for action in actions {
        let key = (
            action.tool_name.as_str(),
            action.repeated_narration.as_str(),
        );
        if let Some(index) = indexes.get(&key).copied() {
            grouped[index].1 += 1;
        } else {
            indexes.insert(key, grouped.len());
            grouped.push((action, 1));
        }
    }

    let phrases = grouped
        .iter()
        .take(2)
        .map(|(action, count)| {
            if *count == 1 {
                action.narration.clone()
            } else {
                format_repeated_action(&action.repeated_narration, *count, locale)
            }
        })
        .collect::<Vec<_>>();
    let omitted_count = grouped
        .iter()
        .skip(2)
        .map(|(_, count)| count)
        .sum::<usize>();

    match phrases.as_slice() {
        [] => strings.working.to_string(),
        [only] => only.clone(),
        [first, second] if omitted_count == 0 => match resolve_backend_locale(locale) {
            BackendLocale::Uk => format!("{first} і {second}"),
            BackendLocale::En => format!("{first} and {second}"),
        },
        [first, second, ..] => {
            let more = format_more_actions(locale, omitted_count);
            format!("{first}, {second}, {more}")
        }
    }
}

fn format_repeated_action(action: &str, count: usize, locale: Option<&str>) -> String {
    match resolve_backend_locale(locale) {
        BackendLocale::En if count == 2 => format!("{action} twice"),
        BackendLocale::En => format!("{action} {count} times"),
        BackendLocale::Uk if count == 2 => format!("{action} двічі"),
        BackendLocale::Uk => {
            let suffix = if (11..=14).contains(&(count % 100)) {
                "разів"
            } else {
                match count % 10 {
                    2..=4 => "рази",
                    _ => "разів",
                }
            };
            format!("{action} {count} {suffix}")
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
    fn list_directory_helper_without_store_echoes_legacy_alias() {
        let a = args(json!({ "path": "/workspace/crates" }));
        assert_eq!(
            narrate_list_directory(
                &a,
                ToolNarrationPhase::Completed,
                None,
                ToolNarrationContext::default()
            ),
            "Listed files in /workspace/crates"
        );
    }

    #[test]
    fn list_directory_helper_uses_store_display_path() {
        use crate::error::Result;
        use crate::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
        use crate::typed_id::SessionId;
        use async_trait::async_trait;

        struct HostBackedStore {
            root: String,
        }

        #[async_trait]
        impl SessionFileSystem for HostBackedStore {
            fn display_root(&self) -> String {
                self.root.clone()
            }

            fn display_path(&self, path: &str) -> String {
                if path == "/" || path == "/workspace" || path == "." {
                    return self.root.clone();
                }
                if path == self.root || path.starts_with(&format!("{}/", self.root)) {
                    return path.to_string();
                }
                if let Some(rest) = path.strip_prefix("/workspace/") {
                    return format!("{}/{}", self.root.trim_end_matches('/'), rest);
                }
                if path.starts_with('/') {
                    return format!(
                        "{}/{}",
                        self.root.trim_end_matches('/'),
                        path.trim_start_matches('/')
                    );
                }
                format!("{}/{}", self.root.trim_end_matches('/'), path)
            }

            fn is_mount_resolver(&self) -> bool {
                false
            }

            async fn read_file(&self, _: SessionId, _: &str) -> Result<Option<SessionFile>> {
                Ok(None)
            }
            async fn write_file(
                &self,
                _: SessionId,
                _: &str,
                _: &str,
                _: &str,
            ) -> Result<SessionFile> {
                Err(anyhow::anyhow!("stub").into())
            }
            async fn delete_file(&self, _: SessionId, _: &str, _: bool) -> Result<bool> {
                Ok(false)
            }
            async fn list_directory(&self, _: SessionId, _: &str) -> Result<Vec<FileInfo>> {
                Ok(vec![])
            }
            async fn stat_file(&self, _: SessionId, _: &str) -> Result<Option<FileStat>> {
                Ok(None)
            }
            async fn grep_files(
                &self,
                _: SessionId,
                _: &str,
                _: Option<&str>,
            ) -> Result<Vec<GrepMatch>> {
                Ok(vec![])
            }
            async fn create_directory(&self, _: SessionId, _: &str) -> Result<FileInfo> {
                Err(anyhow::anyhow!("stub").into())
            }
        }

        let store = HostBackedStore {
            root: "/repo".to_string(),
        };
        let ctx = ToolNarrationContext::new(Some(&store));
        let workspace_alias = args(json!({ "path": "/workspace/crates" }));
        assert_eq!(
            narrate_list_directory(&workspace_alias, ToolNarrationPhase::Started, None, ctx),
            "Listing files in /repo/crates"
        );
        assert_eq!(
            narrate_list_directory(&workspace_alias, ToolNarrationPhase::Completed, None, ctx),
            "Listed files in /repo/crates"
        );
        assert_eq!(
            narrate_list_directory(&workspace_alias, ToolNarrationPhase::Failed, None, ctx),
            "Failed to list files in /repo/crates"
        );

        let relative = args(json!({ "path": "crates" }));
        assert_eq!(
            narrate_list_directory(&relative, ToolNarrationPhase::Completed, None, ctx),
            "Listed files in /repo/crates"
        );

        let host_absolute = args(json!({ "path": "/repo/crates" }));
        assert_eq!(
            narrate_list_directory(&host_absolute, ToolNarrationPhase::Completed, None, ctx),
            "Listed files in /repo/crates"
        );

        let root = args(json!({}));
        assert_eq!(
            narrate_list_directory(&root, ToolNarrationPhase::Completed, None, ctx),
            "Listed files in /repo"
        );
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
            "Отримав URL: example.com/page"
        );
    }

    #[test]
    fn web_fetch_helper_narrates_download_when_save_path_is_present() {
        let a = args(json!({
            "url": "https://example.com/file?token=abc",
            "save_to_file": "/downloads/file"
        }));
        assert_eq!(
            narrate_web_fetch(&a, ToolNarrationPhase::Started, None),
            "Downloading URL: example.com/file"
        );
        assert_eq!(
            narrate_web_fetch(&a, ToolNarrationPhase::Completed, None),
            "Downloaded URL: example.com/file"
        );
        assert_eq!(
            narrate_web_fetch(&a, ToolNarrationPhase::Failed, None),
            "Could not download URL: example.com/file"
        );
        assert_eq!(
            narrate_web_fetch(&a, ToolNarrationPhase::Completed, Some("uk")),
            "Завантажив URL: example.com/file"
        );

        let blank = args(json!({
            "url": "https://example.com/file",
            "save_to_file": "   "
        }));
        assert_eq!(
            narrate_web_fetch(&blank, ToolNarrationPhase::Completed, None),
            "Fetched URL: example.com/file"
        );
    }

    #[test]
    fn url_display_strips_embedded_credentials() {
        assert_eq!(
            url_display("https://user:pass@example.com/path?token=abc"),
            "example.com/path"
        );
        assert_eq!(url_display("https://user:pass@example.com"), "example.com");
        assert_eq!(
            url_display("//user:pass@example.com/path?token=abc#frag"),
            "example.com/path"
        );
        assert_eq!(
            url_display("https:///user:pass@example.com/path?token=abc#frag"),
            "example.com/path"
        );
    }

    #[test]
    fn web_fetch_helper_falls_back_to_bare_verb_for_malformed_url() {
        // When the URL cannot be parsed (no scheme) or has no host, narration
        // must fall back to the bare verb rather than echoing any substring of
        // the argument, which can carry credential-like or secret material.
        for raw in [
            "not a url with secret-token inside",
            "javascript:alert('user:s3cret@host')",
            "user:s3cret-pass@no-scheme/path",
        ] {
            let a = args(json!({ "url": raw }));
            let started = narrate_web_fetch(&a, ToolNarrationPhase::Started, None);
            let completed = narrate_web_fetch(&a, ToolNarrationPhase::Completed, None);
            assert_eq!(started, "Fetching URL", "input: {raw}");
            assert_eq!(completed, "Fetched URL", "input: {raw}");
            assert!(
                !started.contains("secret"),
                "leaked secret for input: {raw}"
            );
            assert!(
                !started.contains("s3cret"),
                "leaked secret for input: {raw}"
            );
            assert!(
                !completed.contains("s3cret"),
                "leaked secret for input: {raw}"
            );
        }
    }

    #[test]
    fn provider_search_never_leaks_secret() {
        let a = args(json!({ "token": "super-secret" }));
        assert_eq!(
            narrate_provider_search(&a, ToolNarrationPhase::Started, None),
            "Searching"
        );
    }

    #[test]
    fn secret_store_uses_real_verb_conjugations() {
        let get = json!({ "operation": "get", "name": "OPENAI_API_KEY" });
        let set = json!({ "operation": "set", "name": "OPENAI_API_KEY" });
        assert_eq!(
            narrate_secret_store(&get, "secret store", ToolNarrationPhase::Started, None),
            "Getting OPENAI_API_KEY"
        );
        assert_eq!(
            narrate_secret_store(&get, "secret store", ToolNarrationPhase::Completed, None),
            "Got OPENAI_API_KEY"
        );
        assert_eq!(
            narrate_secret_store(&set, "secret store", ToolNarrationPhase::Started, None),
            "Setting OPENAI_API_KEY"
        );
        assert_eq!(
            narrate_secret_store(&set, "secret store", ToolNarrationPhase::Completed, None),
            "Set OPENAI_API_KEY"
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
    fn write_session_title_all_phases_and_bounded_detail() {
        let a = args(json!({ "title": "Improve tool narration" }));
        assert_eq!(
            narrate_write_session_title(&a, ToolNarrationPhase::Started, None),
            "Updating session title: Improve tool narration"
        );
        assert_eq!(
            narrate_write_session_title(&a, ToolNarrationPhase::Completed, None),
            "Updated session title: Improve tool narration"
        );
        assert_eq!(
            narrate_write_session_title(&a, ToolNarrationPhase::Failed, None),
            "Failed to update session title: Improve tool narration"
        );
        // Ukrainian must not mix languages.
        assert_eq!(
            narrate_write_session_title(&a, ToolNarrationPhase::Completed, Some("uk")),
            "Оновив назву сесії: Improve tool narration"
        );
    }

    #[test]
    fn write_session_title_truncates_long_title_and_drops_missing() {
        let long = "x".repeat(120);
        let a = args(json!({ "title": long }));
        let narration = narrate_write_session_title(&a, ToolNarrationPhase::Started, None);
        // 48-char cap + ellipsis, plus the phase-specific label.
        assert!(narration.starts_with("Updating session title: "));
        assert!(narration.ends_with("..."));
        assert!(
            narration.chars().count() <= "Updating session title: ".chars().count() + 48 + 3,
            "narration too long: {narration}"
        );

        // No title → bare verb, no dangling separator.
        let empty = args(json!({}));
        assert_eq!(
            narrate_write_session_title(&empty, ToolNarrationPhase::Started, None),
            "Updating session title"
        );
    }

    #[test]
    fn get_session_info_all_phases() {
        assert_eq!(
            narrate_get_session_info(ToolNarrationPhase::Started, None),
            "Reading session info"
        );
        assert_eq!(
            narrate_get_session_info(ToolNarrationPhase::Completed, None),
            "Read session info"
        );
        assert_eq!(
            narrate_get_session_info(ToolNarrationPhase::Failed, None),
            "Failed to read session info"
        );
    }

    #[test]
    fn session_schedule_family_labels_and_detail() {
        let a = args(json!({ "description": "Run nightly backup" }));
        assert_eq!(
            narrate_session_schedule("create_schedule", &a, ToolNarrationPhase::Completed, None),
            Some("Created schedule: Run nightly backup".to_string())
        );
        assert_eq!(
            narrate_session_schedule(
                "cancel_schedule",
                &json!({}),
                ToolNarrationPhase::Started,
                None
            ),
            Some("Cancelling schedule".to_string())
        );
        assert_eq!(
            narrate_session_schedule(
                "list_schedules",
                &json!({}),
                ToolNarrationPhase::Failed,
                None
            ),
            Some("Failed to list schedules".to_string())
        );
        assert_eq!(
            narrate_session_schedule(
                "not_a_schedule",
                &json!({}),
                ToolNarrationPhase::Started,
                None
            ),
            None
        );
    }

    #[test]
    fn session_task_family_labels_and_detail() {
        let a = args(json!({ "task_id": "task_abc" }));
        assert_eq!(
            narrate_session_task("get_task", &a, ToolNarrationPhase::Completed, None),
            Some("Read task: task_abc".to_string())
        );
        assert_eq!(
            narrate_session_task("list_tasks", &json!({}), ToolNarrationPhase::Started, None),
            Some("Listing tasks".to_string())
        );
        // No task_id → bare verb.
        assert_eq!(
            narrate_session_task("cancel_task", &json!({}), ToolNarrationPhase::Started, None),
            Some("Cancelling task".to_string())
        );
        assert_eq!(
            narrate_session_task("wait_task", &a, ToolNarrationPhase::Failed, None),
            Some("Could not wait for task: task_abc".to_string())
        );
        assert_eq!(
            narrate_session_task("nope", &json!({}), ToolNarrationPhase::Started, None),
            None
        );
    }

    #[test]
    fn sandbox_status_and_manage() {
        assert_eq!(
            narrate_sandbox_status(ToolNarrationPhase::Completed, None),
            "Checked sandbox status"
        );
        let a = args(json!({ "action": "pause" }));
        assert_eq!(
            narrate_sandbox_manage(&a, ToolNarrationPhase::Started, None),
            "Managing sandbox: pause"
        );
        assert_eq!(
            narrate_sandbox_manage(&json!({}), ToolNarrationPhase::Failed, None),
            "Failed to manage sandbox"
        );
    }

    #[test]
    fn sql_family_omits_statement() {
        let a = args(json!({ "sql": "SELECT * FROM users WHERE api_token = 'SECRET42'" }));
        assert_eq!(
            narrate_sql("sql_query", &a, ToolNarrationPhase::Completed, None),
            Some("Queried SQL".to_string())
        );
        assert_eq!(
            narrate_sql("sql_execute", &json!({}), ToolNarrationPhase::Started, None),
            Some("Running SQL".to_string())
        );
        assert_eq!(
            narrate_sql("sql_schema", &json!({}), ToolNarrationPhase::Started, None),
            Some("Reading database schema".to_string())
        );
        assert_eq!(
            narrate_sql("other", &json!({}), ToolNarrationPhase::Started, None),
            None
        );
    }

    #[test]
    fn search_knowledge_and_history_never_leak_secret() {
        let query = args(json!({ "query": "orders" }));
        assert_eq!(
            narrate_search_knowledge(&query, ToolNarrationPhase::Completed, None),
            "Searched knowledge: orders"
        );
        assert_eq!(
            narrate_query_history(&query, ToolNarrationPhase::Completed, None),
            "Searched history: orders"
        );
        // A secret-named field is never rendered even if it is the only arg.
        let secret = args(json!({ "api_key": "sk-live-123" }));
        assert_eq!(
            narrate_search_knowledge(&secret, ToolNarrationPhase::Started, None),
            "Searching knowledge"
        );
    }

    #[test]
    fn skill_helper_covers_write_skill() {
        assert_eq!(
            narrate_skill(
                "write_skill",
                &json!({ "name": "pdf" }),
                ToolNarrationPhase::Completed,
                None
            ),
            Some("Wrote skill: pdf".to_string())
        );
    }

    #[test]
    fn delegation_result_family() {
        assert_eq!(
            narrate_delegation_result("report_result", ToolNarrationPhase::Completed, None),
            Some("Reported result".to_string())
        );
        assert_eq!(
            narrate_delegation_result("report_task_progress", ToolNarrationPhase::Started, None),
            Some("Reporting progress".to_string())
        );
        assert_eq!(
            narrate_delegation_result("other", ToolNarrationPhase::Started, None),
            None
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

    fn group_action(key: &str, narration: &str, repeated_narration: &str) -> GroupHeadlineAction {
        GroupHeadlineAction {
            tool_name: key.to_string(),
            narration: narration.to_string(),
            repeated_narration: repeated_narration.to_string(),
        }
    }

    #[test]
    fn group_headline_preserves_one_action_narration() {
        let actions = [group_action(
            "grep_files",
            "Searched files for full_name",
            "Searched files",
        )];

        assert_eq!(
            summarize_group_actions(&actions, None),
            "Searched files for full_name"
        );
    }

    #[test]
    fn group_headline_summarizes_two_repeated_actions() {
        let actions = [
            group_action(
                "grep_files",
                "Searched files for full_name",
                "Searched files",
            ),
            group_action("grep_files", "Searched files for login", "Searched files"),
        ];

        assert_eq!(
            summarize_group_actions(&actions, None),
            "Searched files twice"
        );
    }

    #[test]
    fn group_headline_summarizes_more_than_two_repeated_actions() {
        let actions = [
            group_action("grep_files", "Searched files for one", "Searched files"),
            group_action("grep_files", "Searched files for two", "Searched files"),
            group_action("grep_files", "Searched files for three", "Searched files"),
        ];

        assert_eq!(
            summarize_group_actions(&actions, None),
            "Searched files 3 times"
        );
    }

    #[test]
    fn group_headline_localizes_repeated_action_counts() {
        let three_actions = [
            group_action("grep_files", "Шукав у файлах: один", "Шукав у файлах"),
            group_action("grep_files", "Шукав у файлах: два", "Шукав у файлах"),
            group_action("grep_files", "Шукав у файлах: три", "Шукав у файлах"),
        ];
        let twelve_actions = (0..12)
            .map(|index| {
                group_action(
                    "grep_files",
                    &format!("Шукав у файлах: {index}"),
                    "Шукав у файлах",
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(
            summarize_group_actions(&three_actions, Some("uk")),
            "Шукав у файлах 3 рази"
        );
        assert_eq!(
            summarize_group_actions(&twelve_actions, Some("uk")),
            "Шукав у файлах 12 разів"
        );
    }

    #[test]
    fn group_headline_preserves_mixed_actions_and_bounds_the_fallback() {
        let actions = [
            group_action("grep_files", "Searched files for one", "Searched files"),
            group_action("grep_files", "Searched files for two", "Searched files"),
            group_action("read_file", "Read AGENTS.md", "Read file"),
            group_action("search_web", "Searched web for docs", "Searched web"),
            group_action("search_web", "Searched web for examples", "Searched web"),
        ];

        assert_eq!(
            summarize_group_actions(&actions, None),
            "Searched files twice, Read AGENTS.md, and 2 more actions"
        );
    }
}
