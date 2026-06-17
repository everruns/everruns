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

fn arg_str<'a>(arguments: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .find_map(|key| arguments.get(*key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn basename(path: &str) -> &str {
    let trimmed = path.trim_end_matches('/');
    trimmed
        .rsplit('/')
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or(path)
}

fn truncate(value: &str, max_len: usize) -> String {
    let clean = value.trim();
    if clean.chars().count() <= max_len {
        return clean.to_string();
    }

    let truncated: String = clean.chars().take(max_len).collect();
    format!("{truncated}...")
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

/// Map a CRUD operation to (started, completed, failed) verb forms.
fn operation_verbs(operation: &str) -> (&str, &str, &str) {
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
/// → "Creating agent: Neon Cartographer" / "Created agent: Neon Cartographer"
fn operation_narration(noun: &str, arguments: &Value, phase: ToolNarrationPhase) -> Option<String> {
    let operation = arg_str(arguments, &["operation", "action"])?;
    let (started, completed, failed) = operation_verbs(operation);
    let name =
        arg_str(arguments, &["display_name", "name", "title", "new_name"]).map(|v| truncate(v, 40));
    let target = match name {
        Some(name) => format!("{noun}: {name}"),
        None => noun.to_string(),
    };
    Some(generic_phrase(
        started,
        completed,
        failed,
        Some(target),
        phase,
    ))
}

fn generic_phrase(
    verb_started: &str,
    verb_completed: &str,
    verb_failed: &str,
    target: Option<String>,
    phase: ToolNarrationPhase,
) -> String {
    let verb = match phase {
        ToolNarrationPhase::Started => verb_started,
        ToolNarrationPhase::Waiting => verb_started,
        ToolNarrationPhase::Completed => verb_completed,
        ToolNarrationPhase::Failed => verb_failed,
    };

    match target {
        Some(target) if !target.is_empty() => format!("{verb} {target}"),
        _ => verb.to_string(),
    }
}

/// `"{verb}: {value}"` when a value is present, otherwise the bare `"{verb}"`.
/// This is the neutral "Verb: argument" style used by most common tools
/// (e.g. "Search tools: router", "Activate skill: ship").
fn labeled_phrase(
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
/// generic rules that scan loosely-named arguments can't leak credentials.
fn safe_arg_str<'a>(arguments: &'a Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter()
        .filter(|key| !is_secret_key(key))
        .find_map(|key| arguments.get(*key))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

/// Hook identifier for hook tools: prefers a top-level `id`, falling back to a
/// nested `spec.id`.
fn hook_id(arguments: &Value) -> Option<String> {
    safe_arg_str(arguments, &["id"])
        .or_else(|| {
            arguments
                .get("spec")
                .and_then(|spec| spec.get("id"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .map(|id| truncate(id, 48))
}

/// Display form for a URL argument: host + path, with the scheme, query
/// string, and fragment stripped (the query may carry tokens). Truncated.
fn url_display(url: &str) -> String {
    let without_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let host_path = without_scheme
        .split(['?', '#'])
        .next()
        .unwrap_or(without_scheme)
        .trim_end_matches('/');
    let cleaned = if host_path.is_empty() {
        without_scheme
    } else {
        host_path
    };
    truncate(cleaned, 48)
}

fn is_shell_exec_tool(name: &str) -> bool {
    matches!(
        name,
        "bash"
            | "shell"
            | "run_shell"
            | "daytona_exec"
            | "sandbox_exec"
            | "e2b_exec"
            | "deno_exec"
            | "docker_exec"
            | "sprites_exec"
    )
}

fn ukrainian_phrase(
    _tool_def: Option<&ToolDefinition>,
    tool_call: &ToolCall,
    fallback_name: &str,
    phase: ToolNarrationPhase,
) -> String {
    let args = &tool_call.arguments;
    match tool_call.name.as_str() {
        name if is_shell_exec_tool(name) => {
            let command = arg_str(args, &["commands", "command"])
                .map(|value| format!("`{}`", truncate(value, 48)))
                .unwrap_or_else(|| fallback_name.to_string());
            match phase {
                ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => {
                    format!("Запускаю {command}")
                }
                ToolNarrationPhase::Completed => format!("Запустив {command}"),
                ToolNarrationPhase::Failed => format!("Не вдалося запустити {command}"),
            }
        }
        "read_file" | "session_read_file" => {
            let target = arg_str(args, &["path"]).map(|path| basename(path).to_string());
            match phase {
                ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => match target {
                    Some(target) => format!("Читаю {target}"),
                    None => "Читаю файл".to_string(),
                },
                ToolNarrationPhase::Completed => match target {
                    Some(target) => format!("Прочитав {target}"),
                    None => "Прочитав файл".to_string(),
                },
                ToolNarrationPhase::Failed => match target {
                    Some(target) => format!("Не вдалося прочитати {target}"),
                    None => "Не вдалося прочитати файл".to_string(),
                },
            }
        }
        "read_many_files" => match phase {
            ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => {
                "Читаю кілька файлів".to_string()
            }
            ToolNarrationPhase::Completed => "Прочитав кілька файлів".to_string(),
            ToolNarrationPhase::Failed => "Не вдалося прочитати кілька файлів".to_string(),
        },
        "list_directory" | "list_files" => {
            let target = location_phrase(args, Some("uk"));
            match phase {
                ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => {
                    format!("Переглядаю файли у {target}")
                }
                ToolNarrationPhase::Completed => format!("Переглянув файли у {target}"),
                ToolNarrationPhase::Failed => format!("Не вдалося переглянути файли у {target}"),
            }
        }
        "grep_files" => {
            let pattern = arg_str(args, &["pattern"]).map(|pattern| truncate(pattern, 36));
            match phase {
                ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => match pattern {
                    Some(pattern) => format!("Шукаю `{pattern}` у файлах"),
                    None => "Шукаю у файлах".to_string(),
                },
                ToolNarrationPhase::Completed => match pattern {
                    Some(pattern) => format!("Знайшов `{pattern}` у файлах"),
                    None => "Завершив пошук у файлах".to_string(),
                },
                ToolNarrationPhase::Failed => match pattern {
                    Some(pattern) => format!("Не вдалося знайти `{pattern}` у файлах"),
                    None => "Не вдалося виконати пошук у файлах".to_string(),
                },
            }
        }
        "search" | "search_web" => {
            let query = arg_str(args, &["query", "q", "search"]).map(|query| truncate(query, 48));
            match phase {
                ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => match query {
                    Some(query) => format!("Шукаю у вебі: {query}"),
                    None => "Шукаю у вебі".to_string(),
                },
                ToolNarrationPhase::Completed => match query {
                    Some(query) => format!("Завершив пошук у вебі: {query}"),
                    None => "Завершив пошук у вебі".to_string(),
                },
                ToolNarrationPhase::Failed => match query {
                    Some(query) => format!("Не вдалося знайти у вебі: {query}"),
                    None => "Не вдалося виконати пошук у вебі".to_string(),
                },
            }
        }
        name if name.ends_with("__search") => {
            let query = arg_str(args, &["query", "q", "search"]).map(|query| truncate(query, 48));
            match phase {
                ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => match query {
                    Some(query) => format!("Шукаю: {query}"),
                    None => "Шукаю".to_string(),
                },
                ToolNarrationPhase::Completed => match query {
                    Some(query) => format!("Завершив пошук: {query}"),
                    None => "Завершив пошук".to_string(),
                },
                ToolNarrationPhase::Failed => match query {
                    Some(query) => format!("Не вдалося знайти: {query}"),
                    None => "Не вдалося виконати пошук".to_string(),
                },
            }
        }
        "write_file" => {
            let target = arg_str(args, &["path"]).map(|path| basename(path).to_string());
            match phase {
                ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => match target {
                    Some(target) => format!("Записую {target}"),
                    None => "Записую файл".to_string(),
                },
                ToolNarrationPhase::Completed => match target {
                    Some(target) => format!("Записав {target}"),
                    None => "Записав файл".to_string(),
                },
                ToolNarrationPhase::Failed => match target {
                    Some(target) => format!("Не вдалося записати {target}"),
                    None => "Не вдалося записати файл".to_string(),
                },
            }
        }
        "edit_file" | "replace_in_file" => {
            let target = arg_str(args, &["path"]).map(|path| basename(path).to_string());
            match phase {
                ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => match target {
                    Some(target) => format!("Редагую {target}"),
                    None => "Редагую файл".to_string(),
                },
                ToolNarrationPhase::Completed => match target {
                    Some(target) => format!("Відредагував {target}"),
                    None => "Відредагував файл".to_string(),
                },
                ToolNarrationPhase::Failed => match target {
                    Some(target) => format!("Не вдалося відредагувати {target}"),
                    None => "Не вдалося відредагувати файл".to_string(),
                },
            }
        }
        "append_file" => {
            let target = arg_str(args, &["path"]).map(|path| basename(path).to_string());
            match phase {
                ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => match target {
                    Some(target) => format!("Дописую у {target}"),
                    None => "Дописую у файл".to_string(),
                },
                ToolNarrationPhase::Completed => match target {
                    Some(target) => format!("Дописав у {target}"),
                    None => "Дописав у файл".to_string(),
                },
                ToolNarrationPhase::Failed => match target {
                    Some(target) => format!("Не вдалося дописати у {target}"),
                    None => "Не вдалося дописати у файл".to_string(),
                },
            }
        }
        "move_file" => {
            let target = arg_str(args, &["to", "destination", "new_path"])
                .or_else(|| arg_str(args, &["path"]))
                .map(|path| basename(path).to_string());
            match phase {
                ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => match target {
                    Some(target) => format!("Переміщую {target}"),
                    None => "Переміщую файл".to_string(),
                },
                ToolNarrationPhase::Completed => match target {
                    Some(target) => format!("Перемістив {target}"),
                    None => "Перемістив файл".to_string(),
                },
                ToolNarrationPhase::Failed => match target {
                    Some(target) => format!("Не вдалося перемістити {target}"),
                    None => "Не вдалося перемістити файл".to_string(),
                },
            }
        }
        "delete_file" => {
            let target = arg_str(args, &["path"]).map(|path| basename(path).to_string());
            match phase {
                ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => match target {
                    Some(target) => format!("Видаляю {target}"),
                    None => "Видаляю файл".to_string(),
                },
                ToolNarrationPhase::Completed => match target {
                    Some(target) => format!("Видалив {target}"),
                    None => "Видалив файл".to_string(),
                },
                ToolNarrationPhase::Failed => match target {
                    Some(target) => format!("Не вдалося видалити {target}"),
                    None => "Не вдалося видалити файл".to_string(),
                },
            }
        }
        "mkdir" => {
            let target = arg_str(args, &["path"]).map(|path| basename(path).to_string());
            match phase {
                ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => match target {
                    Some(target) => format!("Створюю директорію {target}"),
                    None => "Створюю директорію".to_string(),
                },
                ToolNarrationPhase::Completed => match target {
                    Some(target) => format!("Створив директорію {target}"),
                    None => "Створив директорію".to_string(),
                },
                ToolNarrationPhase::Failed => match target {
                    Some(target) => format!("Не вдалося створити директорію {target}"),
                    None => "Не вдалося створити директорію".to_string(),
                },
            }
        }
        "stat_file" => {
            let target = arg_str(args, &["path"]).map(|path| basename(path).to_string());
            match phase {
                ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => match target {
                    Some(target) => format!("Перевіряю {target}"),
                    None => "Перевіряю файл".to_string(),
                },
                ToolNarrationPhase::Completed => match target {
                    Some(target) => format!("Перевірив {target}"),
                    None => "Перевірив файл".to_string(),
                },
                ToolNarrationPhase::Failed => match target {
                    Some(target) => format!("Не вдалося перевірити {target}"),
                    None => "Не вдалося перевірити файл".to_string(),
                },
            }
        }
        "secret_store" | "kv_store" => {
            let operation = arg_str(args, &["operation"]).unwrap_or("використати");
            let target = arg_str(args, &["name", "key"])
                .map(str::to_string)
                .unwrap_or_else(|| fallback_name.to_string());
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
                ToolNarrationPhase::Failed => {
                    format!("Не вдалося виконати {} {}", operation, target)
                        .trim()
                        .to_string()
                }
            }
        }
        "spawn_subagent" => {
            let target = arg_str(args, &["name"])
                .map(|name| format!("субагента {name}"))
                .unwrap_or_else(|| "субагента".to_string());
            match phase {
                ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => {
                    format!("Запускаю {target}")
                }
                ToolNarrationPhase::Completed => format!("Запустив {target}"),
                ToolNarrationPhase::Failed => format!("Не вдалося запустити {target}"),
            }
        }
        "write_todos" => match phase {
            ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => {
                "Оновлюю список задач".to_string()
            }
            ToolNarrationPhase::Completed => "Оновив список задач".to_string(),
            ToolNarrationPhase::Failed => "Не вдалося оновити список задач".to_string(),
        },
        // Ukrainian operation_narration not yet localized — use generic
        // Ukrainian fallback to avoid mixing English verbs into the UI.
        _ => match phase {
            ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => {
                format!("Запускаю {fallback_name}")
            }
            ToolNarrationPhase::Completed => format!("Запустив {fallback_name}"),
            ToolNarrationPhase::Failed => {
                format!("Не вдалося запустити {fallback_name}")
            }
        },
    }
}

pub fn render_tool_narration_with_locale(
    tool_def: Option<&ToolDefinition>,
    tool_call: &ToolCall,
    phase: ToolNarrationPhase,
    locale: Option<&str>,
) -> String {
    let args = &tool_call.arguments;
    let fallback_name = display_name(tool_def, tool_call, locale);

    if resolve_backend_locale(locale) == BackendLocale::Uk {
        return ukrainian_phrase(tool_def, tool_call, &fallback_name, phase);
    }

    match tool_call.name.as_str() {
        name if is_shell_exec_tool(name) => {
            let command = arg_str(args, &["commands", "command"])
                .map(|value| format!("`{}`", truncate(value, 48)))
                .unwrap_or_else(|| fallback_name.clone());
            generic_phrase("Running", "Ran", "Failed to run", Some(command), phase)
        }
        "read_file" | "session_read_file" => {
            let target = arg_str(args, &["path"]).map(|path| basename(path).to_string());
            generic_phrase("Reading", "Read", "Failed to read", target, phase)
        }
        "read_many_files" => generic_phrase(
            "Reading",
            "Read",
            "Failed to read",
            Some("multiple files".to_string()),
            phase,
        ),
        "list_directory" | "list_files" => generic_phrase(
            "Listing files in",
            "Listed files in",
            "Failed to list files in",
            Some(location_phrase(args, locale)),
            phase,
        ),
        "grep_files" => {
            let target = arg_str(args, &["pattern"])
                .map(|pattern| format!("files for {}", truncate(pattern, 36)))
                .unwrap_or_else(|| "files".to_string());
            generic_phrase(
                "Searching",
                "Searched",
                "Failed to search",
                Some(target),
                phase,
            )
        }
        "search" | "search_web" => {
            let target = arg_str(args, &["query", "q", "search"])
                .map(|query| format!("web for {}", truncate(query, 48)))
                .unwrap_or_else(|| "web".to_string());
            generic_phrase(
                "Searching",
                "Searched",
                "Failed to search",
                Some(target),
                phase,
            )
        }
        name if name.ends_with("__search") => {
            let target = arg_str(args, &["query", "q", "search"])
                .map(|query| truncate(query, 48))
                .unwrap_or_else(|| "query".to_string());
            generic_phrase(
                "Searching",
                "Searched",
                "Failed to search",
                Some(target),
                phase,
            )
        }
        "write_file" => {
            let target = arg_str(args, &["path"]).map(|path| basename(path).to_string());
            generic_phrase("Writing", "Wrote", "Failed to write", target, phase)
        }
        "edit_file" | "replace_in_file" => {
            let target = arg_str(args, &["path"]).map(|path| basename(path).to_string());
            generic_phrase("Editing", "Edited", "Failed to edit", target, phase)
        }
        "append_file" => {
            let target = arg_str(args, &["path"]).map(|path| basename(path).to_string());
            generic_phrase(
                "Appending to",
                "Appended to",
                "Failed to append to",
                target,
                phase,
            )
        }
        "move_file" => {
            let target = arg_str(args, &["to", "destination", "new_path"])
                .or_else(|| arg_str(args, &["path"]))
                .map(|path| basename(path).to_string());
            generic_phrase("Moving", "Moved", "Failed to move", target, phase)
        }
        "delete_file" => {
            let target = arg_str(args, &["path"]).map(|path| basename(path).to_string());
            generic_phrase("Deleting", "Deleted", "Failed to delete", target, phase)
        }
        "mkdir" => {
            let target = arg_str(args, &["path"]).map(|path| basename(path).to_string());
            generic_phrase(
                "Creating directory",
                "Created directory",
                "Failed to create directory",
                target,
                phase,
            )
        }
        "stat_file" => {
            let target = arg_str(args, &["path"]).map(|path| basename(path).to_string());
            generic_phrase("Checking", "Checked", "Failed to check", target, phase)
        }
        "secret_store" | "kv_store" => {
            let operation = arg_str(args, &["operation"]).unwrap_or("use");
            let target = arg_str(args, &["name", "key"])
                .map(str::to_string)
                .or_else(|| Some(fallback_name.clone()));
            let started = format!("{}ing", title_case(operation));
            let completed = if operation.eq_ignore_ascii_case("list") {
                format!(
                    "Listed {target}",
                    target = target.clone().unwrap_or_default()
                )
            } else {
                format!(
                    "{} {}",
                    title_case(operation),
                    target.clone().unwrap_or_default()
                )
            };
            match phase {
                ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => {
                    format!("{} {}", started, target.unwrap_or_default())
                        .trim()
                        .to_string()
                }
                ToolNarrationPhase::Completed => completed.trim().to_string(),
                ToolNarrationPhase::Failed => format!(
                    "Failed to {} {}",
                    operation.to_lowercase(),
                    target.unwrap_or_default()
                )
                .trim()
                .to_string(),
            }
        }
        "spawn_subagent" => {
            let target = arg_str(args, &["name"])
                .map(|name| format!("{name} subagent"))
                .or_else(|| Some("subagent".to_string()));
            generic_phrase("Launching", "Launched", "Failed to launch", target, phase)
        }
        "write_todos" => generic_phrase(
            "Updating",
            "Updated",
            "Failed to update",
            Some("task list".to_string()),
            phase,
        ),
        // --- Tool discovery and skills ---
        "tool_search" => {
            let value = safe_arg_str(args, &["query"]).map(|q| truncate(q, 64));
            labeled_phrase(
                "Search tools",
                "Searched tools",
                "Could not search tools",
                value,
                phase,
            )
        }
        "activate_skill" => {
            let value = safe_arg_str(args, &["name", "skill", "id"]).map(|v| truncate(v, 48));
            labeled_phrase(
                "Activate skill",
                "Activated skill",
                "Could not activate skill",
                value,
                phase,
            )
        }
        "read_skill" => {
            let value = safe_arg_str(args, &["name", "skill", "id"]).map(|v| truncate(v, 48));
            labeled_phrase(
                "Read skill",
                "Read skill",
                "Could not read skill",
                value,
                phase,
            )
        }
        "list_skills" => generic_phrase(
            "Listing skills",
            "Listed skills",
            "Could not list skills",
            None,
            phase,
        ),
        "run_command" | "run_client_command" | "run_yolop_command" => {
            // Slash-command runner. Show the command name only; never dump the
            // free-text `arguments` payload.
            let value = safe_arg_str(args, &["command"]).map(|c| format!("/{}", truncate(c, 48)));
            labeled_phrase(
                "Run command",
                "Ran command",
                "Could not run command",
                value,
                phase,
            )
        }

        // --- Code intelligence ---
        "repo_symbols" => {
            let query = safe_arg_str(args, &["query", "symbol"]).map(|q| truncate(q, 64));
            match query {
                Some(query) => labeled_phrase(
                    "Search symbols",
                    "Searched symbols",
                    "Could not search symbols",
                    Some(query),
                    phase,
                ),
                None => generic_phrase(
                    "Listing symbols",
                    "Listed symbols",
                    "Could not list symbols",
                    None,
                    phase,
                ),
            }
        }
        "repo_map" => {
            let value = safe_arg_str(args, &["path"]).map(|p| truncate(p, 48));
            labeled_phrase(
                "Build repo map",
                "Built repo map",
                "Could not build repo map",
                value,
                phase,
            )
        }
        "ast_grep" => {
            let value = safe_arg_str(args, &["pattern"]).map(|p| truncate(p, 64));
            labeled_phrase(
                "Search code",
                "Searched code",
                "Could not search code",
                value,
                phase,
            )
        }
        "duckduckgo_search" => {
            let value = safe_arg_str(args, &["query", "q", "search"]).map(|q| truncate(q, 64));
            labeled_phrase(
                "Search DuckDuckGo",
                "Searched DuckDuckGo",
                "Could not search DuckDuckGo",
                value,
                phase,
            )
        }
        "web_fetch" => {
            let value = safe_arg_str(args, &["url", "uri"]).map(url_display);
            labeled_phrase(
                "Fetch URL",
                "Fetched URL",
                "Could not fetch URL",
                value,
                phase,
            )
        }

        // --- Background task family ---
        "background_run" => {
            let value = safe_arg_str(args, &["command", "cmd"]).map(|c| truncate(c, 64));
            labeled_phrase(
                "Start background command",
                "Started background command",
                "Could not start background command",
                value,
                phase,
            )
        }
        "background_output" => {
            let value = safe_arg_str(args, &["task_id", "id"]).map(|v| truncate(v, 48));
            labeled_phrase(
                "Read background output",
                "Read background output",
                "Could not read background output",
                value,
                phase,
            )
        }
        "background_list" => generic_phrase(
            "Listing background tasks",
            "Listed background tasks",
            "Could not list background tasks",
            None,
            phase,
        ),
        "background_cancel" => {
            let value = safe_arg_str(args, &["task_id", "id"]).map(|v| truncate(v, 48));
            labeled_phrase(
                "Cancel background task",
                "Canceled background task",
                "Could not cancel background task",
                value,
                phase,
            )
        }
        // Never echo the agent prompt/instruction.
        "background_agent" => generic_phrase(
            "Starting background agent",
            "Started background agent",
            "Could not start background agent",
            None,
            phase,
        ),

        // --- Config and personalization ---
        "get_config" => {
            let value = safe_arg_str(args, &["key"]).map(|k| truncate(k, 48));
            labeled_phrase(
                "Read config",
                "Read config",
                "Could not read config",
                value,
                phase,
            )
        }
        "set_config" => {
            let value = safe_arg_str(args, &["key"]).map(|k| truncate(k, 48));
            labeled_phrase(
                "Update config",
                "Updated config",
                "Could not update config",
                value,
                phase,
            )
        }
        "remember" => {
            let value = safe_arg_str(args, &["title"]).map(|t| truncate(t, 48));
            labeled_phrase(
                "Save memory",
                "Saved memory",
                "Could not save memory",
                value,
                phase,
            )
        }
        "recall" => {
            let query = safe_arg_str(args, &["query"]).map(|q| truncate(q, 64));
            match query {
                Some(query) => labeled_phrase(
                    "Search memory",
                    "Searched memory",
                    "Could not search memory",
                    Some(query),
                    phase,
                ),
                None => generic_phrase(
                    "Reading memory",
                    "Read memory",
                    "Could not read memory",
                    None,
                    phase,
                ),
            }
        }
        "forget" => {
            let value = safe_arg_str(args, &["title", "id"]).map(|v| truncate(v, 48));
            labeled_phrase(
                "Delete memory",
                "Deleted memory",
                "Could not delete memory",
                value,
                phase,
            )
        }

        // --- Hooks ---
        "list_hooks" => generic_phrase(
            "Listing hooks",
            "Listed hooks",
            "Could not list hooks",
            None,
            phase,
        ),
        "validate_hook" => {
            let value = hook_id(args);
            labeled_phrase(
                "Validate hook",
                "Validated hook",
                "Could not validate hook",
                value,
                phase,
            )
        }
        "upsert_hook" => {
            let value = hook_id(args);
            labeled_phrase(
                "Save hook",
                "Saved hook",
                "Could not save hook",
                value,
                phase,
            )
        }
        "remove_hook" => {
            let value = hook_id(args);
            labeled_phrase(
                "Remove hook",
                "Removed hook",
                "Could not remove hook",
                value,
                phase,
            )
        }

        // --- Connectors ---
        "list_connectors" => generic_phrase(
            "Listing connectors",
            "Listed connectors",
            "Could not list connectors",
            None,
            phase,
        ),
        "get_connector" => {
            let value = safe_arg_str(args, &["provider", "name"]).map(|v| truncate(v, 48));
            labeled_phrase(
                "Read connector",
                "Read connector",
                "Could not read connector",
                value,
                phase,
            )
        }
        "connect" => {
            let value = safe_arg_str(args, &["provider", "name"]).map(|v| truncate(v, 48));
            labeled_phrase(
                "Connect provider",
                "Connected provider",
                "Could not connect provider",
                value,
                phase,
            )
        }
        "disconnect" => {
            let value = safe_arg_str(args, &["provider", "name"]).map(|v| truncate(v, 48));
            labeled_phrase(
                "Disconnect provider",
                "Disconnected provider",
                "Could not disconnect provider",
                value,
                phase,
            )
        }

        // --- Approval and safety ---
        // Never echo the approved action; it may contain sensitive detail.
        "record_approval" => generic_phrase(
            "Recording approval",
            "Recorded approval",
            "Could not record approval",
            None,
            phase,
        ),
        "set_approval_mode" => {
            let value = safe_arg_str(args, &["mode"]).map(|m| truncate(m, 32));
            labeled_phrase(
                "Set approval mode",
                "Set approval mode",
                "Could not set approval mode",
                value,
                phase,
            )
        }

        // --- Generic shape rules (cover unseen sibling tools by name/argument
        // shape). These run only after every exact rule above misses. ---
        name if name.contains("fetch") => {
            let value = safe_arg_str(args, &["url", "uri"]).map(url_display);
            labeled_phrase(
                "Fetch URL",
                "Fetched URL",
                "Could not fetch URL",
                value,
                phase,
            )
        }
        name if name.contains("symbol") => {
            let value = safe_arg_str(args, &["query", "symbol", "path"]).map(|v| truncate(v, 64));
            labeled_phrase(
                "Search symbols",
                "Searched symbols",
                "Could not search symbols",
                value,
                phase,
            )
        }
        name if name.contains("repo_map") => {
            let value = safe_arg_str(args, &["path"]).map(|p| truncate(p, 48));
            labeled_phrase(
                "Build repo map",
                "Built repo map",
                "Could not build repo map",
                value,
                phase,
            )
        }
        name if name.contains("config") => {
            let value = safe_arg_str(args, &["key"]).map(|k| truncate(k, 48));
            if name.contains("set") || name.contains("update") || name.contains("write") {
                labeled_phrase(
                    "Update config",
                    "Updated config",
                    "Could not update config",
                    value,
                    phase,
                )
            } else {
                labeled_phrase(
                    "Read config",
                    "Read config",
                    "Could not read config",
                    value,
                    phase,
                )
            }
        }
        name if name.ends_with("_search") || name.contains("search") => {
            let value =
                safe_arg_str(args, &["query", "q", "search", "pattern"]).map(|v| truncate(v, 64));
            labeled_phrase("Search", "Searched", "Could not search", value, phase)
        }
        _ => {
            if let Some(narration) = tool_def
                .and_then(|def| def.hints().narration_noun.as_deref())
                .and_then(|noun| operation_narration(noun, args, phase))
            {
                narration
            } else {
                generic_phrase(
                    "Running",
                    "Ran",
                    "Failed to run",
                    Some(fallback_name),
                    phase,
                )
            }
        }
    }
}

pub fn render_group_headline(
    tool_calls: &[ToolCall],
    tool_defs: &[ToolDefinition],
    phase: ToolNarrationPhase,
) -> Option<String> {
    render_group_headline_with_locale(tool_calls, tool_defs, phase, None)
}

pub fn render_tool_narration(
    tool_def: Option<&ToolDefinition>,
    tool_call: &ToolCall,
    phase: ToolNarrationPhase,
) -> String {
    render_tool_narration_with_locale(tool_def, tool_call, phase, None)
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

    use super::{
        ToolNarrationPhase, render_group_headline, render_group_headline_with_locale,
        render_tool_narration, render_tool_narration_with_locale,
    };
    use crate::tool_types::ToolCall;

    #[test]
    fn renders_read_file_narration() {
        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "read_file".to_string(),
            arguments: json!({ "path": "/workspace/AGENTS.md" }),
        };

        assert_eq!(
            render_tool_narration(None, &tool_call, ToolNarrationPhase::Started),
            "Reading AGENTS.md"
        );
        assert_eq!(
            render_tool_narration(None, &tool_call, ToolNarrationPhase::Completed),
            "Read AGENTS.md"
        );
    }

    #[test]
    fn renders_edit_file_narration() {
        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "edit_file".to_string(),
            arguments: json!({ "path": "/workspace/src/main.rs" }),
        };

        assert_eq!(
            render_tool_narration(None, &tool_call, ToolNarrationPhase::Started),
            "Editing main.rs"
        );
        assert_eq!(
            render_tool_narration(None, &tool_call, ToolNarrationPhase::Completed),
            "Edited main.rs"
        );
    }

    #[test]
    fn renders_daytona_exec_narration() {
        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "daytona_exec".to_string(),
            arguments: json!({ "command": "cargo test -p everruns-core" }),
        };

        assert_eq!(
            render_tool_narration(None, &tool_call, ToolNarrationPhase::Started),
            "Running `cargo test -p everruns-core`"
        );
        assert_eq!(
            render_tool_narration(None, &tool_call, ToolNarrationPhase::Completed),
            "Ran `cargo test -p everruns-core`"
        );
    }

    #[test]
    fn renders_sandbox_exec_narration() {
        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "sandbox_exec".to_string(),
            arguments: json!({ "command": "npm test" }),
        };

        assert_eq!(
            render_tool_narration(None, &tool_call, ToolNarrationPhase::Started),
            "Running `npm test`"
        );
        assert_eq!(
            render_tool_narration(None, &tool_call, ToolNarrationPhase::Completed),
            "Ran `npm test`"
        );
    }

    #[test]
    fn renders_bash_narration_with_commands_arg() {
        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "bash".to_string(),
            arguments: json!({ "commands": "echo hi" }),
        };

        assert_eq!(
            render_tool_narration(None, &tool_call, ToolNarrationPhase::Started),
            "Running `echo hi`"
        );
        assert_eq!(
            render_tool_narration(None, &tool_call, ToolNarrationPhase::Completed),
            "Ran `echo hi`"
        );
    }

    #[test]
    fn renders_group_headline_from_multiple_tool_calls() {
        let tool_calls = vec![
            ToolCall {
                id: "call_1".to_string(),
                name: "list_directory".to_string(),
                arguments: json!({ "path": "/workspace" }),
            },
            ToolCall {
                id: "call_2".to_string(),
                name: "read_file".to_string(),
                arguments: json!({ "path": "/workspace/AGENTS.md" }),
            },
            ToolCall {
                id: "call_3".to_string(),
                name: "grep_files".to_string(),
                arguments: json!({ "pattern": "Doppler" }),
            },
        ];

        let headline =
            render_group_headline(&tool_calls, &[], ToolNarrationPhase::Completed).unwrap();

        assert_eq!(
            headline,
            "Listed files in current directory, Read AGENTS.md, and 1 more actions"
        );
    }

    #[test]
    fn renders_ukrainian_read_file_narration() {
        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "read_file".to_string(),
            arguments: json!({ "path": "/workspace/AGENTS.md" }),
        };

        assert_eq!(
            render_tool_narration_with_locale(
                None,
                &tool_call,
                ToolNarrationPhase::Started,
                Some("uk-UA"),
            ),
            "Читаю AGENTS.md"
        );
        assert_eq!(
            render_tool_narration_with_locale(
                None,
                &tool_call,
                ToolNarrationPhase::Completed,
                Some("uk-UA"),
            ),
            "Прочитав AGENTS.md"
        );
    }

    #[test]
    fn renders_ukrainian_sandbox_exec_narration() {
        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "sandbox_exec".to_string(),
            arguments: json!({ "command": "npm test" }),
        };

        assert_eq!(
            render_tool_narration_with_locale(
                None,
                &tool_call,
                ToolNarrationPhase::Started,
                Some("uk-UA"),
            ),
            "Запускаю `npm test`"
        );
        assert_eq!(
            render_tool_narration_with_locale(
                None,
                &tool_call,
                ToolNarrationPhase::Completed,
                Some("uk-UA"),
            ),
            "Запустив `npm test`"
        );
    }

    #[test]
    fn renders_ukrainian_bash_narration_with_commands_arg() {
        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "bash".to_string(),
            arguments: json!({ "commands": "echo hi" }),
        };

        assert_eq!(
            render_tool_narration_with_locale(
                None,
                &tool_call,
                ToolNarrationPhase::Started,
                Some("uk-UA"),
            ),
            "Запускаю `echo hi`"
        );
        assert_eq!(
            render_tool_narration_with_locale(
                None,
                &tool_call,
                ToolNarrationPhase::Completed,
                Some("uk-UA"),
            ),
            "Запустив `echo hi`"
        );
    }

    #[test]
    fn renders_ukrainian_group_headline() {
        let tool_calls = vec![
            ToolCall {
                id: "call_1".to_string(),
                name: "list_directory".to_string(),
                arguments: json!({ "path": "/workspace" }),
            },
            ToolCall {
                id: "call_2".to_string(),
                name: "read_file".to_string(),
                arguments: json!({ "path": "/workspace/AGENTS.md" }),
            },
            ToolCall {
                id: "call_3".to_string(),
                name: "grep_files".to_string(),
                arguments: json!({ "pattern": "Doppler" }),
            },
        ];

        let headline = render_group_headline_with_locale(
            &tool_calls,
            &[],
            ToolNarrationPhase::Completed,
            Some("uk"),
        )
        .unwrap();

        assert_eq!(
            headline,
            "Переглянув файли у поточній директорії, Прочитав AGENTS.md, і ще 1 дію"
        );
    }

    #[test]
    fn narration_noun_produces_operation_based_narration() {
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
    fn narration_noun_without_name_shows_noun_only() {
        use crate::tool_types::{BuiltinTool, ToolDefinition, ToolHints};

        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "manage_agents".to_string(),
            arguments: json!({ "operation": "delete", "agent_id": "agent_123" }),
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
            render_tool_narration(Some(&def), &tool_call, ToolNarrationPhase::Completed),
            "Deleted agent"
        );
    }

    #[test]
    fn narration_noun_without_operation_falls_back() {
        use crate::tool_types::{BuiltinTool, ToolDefinition, ToolHints};

        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "manage_agents".to_string(),
            arguments: json!({ "name": "Test" }),
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

        // No operation arg → falls back to generic
        assert_eq!(
            render_tool_narration(Some(&def), &tool_call, ToolNarrationPhase::Completed),
            "Ran Manage Agents"
        );
    }

    fn call(name: &str, arguments: serde_json::Value) -> ToolCall {
        ToolCall {
            id: "call_1".to_string(),
            name: name.to_string(),
            arguments,
        }
    }

    fn narrate(name: &str, args: serde_json::Value, phase: ToolNarrationPhase) -> String {
        render_tool_narration(None, &call(name, args), phase)
    }

    #[test]
    fn renders_tool_search_narration() {
        assert_eq!(
            narrate(
                "tool_search",
                json!({ "query": "router" }),
                ToolNarrationPhase::Started
            ),
            "Search tools: router"
        );
        assert_eq!(
            narrate(
                "tool_search",
                json!({ "query": "router" }),
                ToolNarrationPhase::Completed
            ),
            "Searched tools: router"
        );
        assert_eq!(
            narrate("tool_search", json!({}), ToolNarrationPhase::Failed),
            "Could not search tools"
        );
    }

    #[test]
    fn renders_skill_family_narration() {
        assert_eq!(
            narrate(
                "activate_skill",
                json!({ "name": "ship" }),
                ToolNarrationPhase::Completed
            ),
            "Activated skill: ship"
        );
        assert_eq!(
            narrate(
                "read_skill",
                json!({ "name": "ship" }),
                ToolNarrationPhase::Started
            ),
            "Read skill: ship"
        );
        assert_eq!(
            narrate("list_skills", json!({}), ToolNarrationPhase::Completed),
            "Listed skills"
        );
    }

    #[test]
    fn renders_run_command_shows_slash_and_hides_arguments() {
        assert_eq!(
            narrate(
                "run_command",
                json!({ "command": "deploy", "arguments": "--prod secret-token" }),
                ToolNarrationPhase::Started
            ),
            "Run command: /deploy"
        );
    }

    #[test]
    fn renders_repo_symbols_search_and_list() {
        assert_eq!(
            narrate(
                "repo_symbols",
                json!({ "query": "Router" }),
                ToolNarrationPhase::Completed
            ),
            "Searched symbols: Router"
        );
        assert_eq!(
            narrate("repo_symbols", json!({}), ToolNarrationPhase::Completed),
            "Listed symbols"
        );
    }

    #[test]
    fn renders_ast_grep_as_code_search() {
        assert_eq!(
            narrate(
                "ast_grep",
                json!({ "pattern": "fn $NAME()", "language": "rust" }),
                ToolNarrationPhase::Started
            ),
            "Search code: fn $NAME()"
        );
    }

    #[test]
    fn renders_web_fetch_strips_scheme_and_query() {
        assert_eq!(
            narrate(
                "web_fetch",
                json!({ "url": "https://example.com/page?token=abc#frag" }),
                ToolNarrationPhase::Completed
            ),
            "Fetched URL: example.com/page"
        );
    }

    #[test]
    fn renders_background_task_family() {
        assert_eq!(
            narrate(
                "background_run",
                json!({ "command": "cargo build" }),
                ToolNarrationPhase::Started
            ),
            "Start background command: cargo build"
        );
        assert_eq!(
            narrate(
                "background_output",
                json!({ "task_id": "task-1" }),
                ToolNarrationPhase::Completed
            ),
            "Read background output: task-1"
        );
        assert_eq!(
            narrate("background_list", json!({}), ToolNarrationPhase::Completed),
            "Listed background tasks"
        );
        assert_eq!(
            narrate(
                "background_cancel",
                json!({ "id": "task-1" }),
                ToolNarrationPhase::Completed
            ),
            "Canceled background task: task-1"
        );
    }

    #[test]
    fn background_agent_never_echoes_instruction() {
        assert_eq!(
            narrate(
                "background_agent",
                json!({ "instruction": "do a very long secret thing" }),
                ToolNarrationPhase::Started
            ),
            "Starting background agent"
        );
    }

    #[test]
    fn renders_config_and_memory_family() {
        assert_eq!(
            narrate(
                "get_config",
                json!({ "key": "providers.openai.model" }),
                ToolNarrationPhase::Completed
            ),
            "Read config: providers.openai.model"
        );
        assert_eq!(
            narrate(
                "set_config",
                json!({ "key": "default_model" }),
                ToolNarrationPhase::Completed
            ),
            "Updated config: default_model"
        );
        assert_eq!(
            narrate(
                "remember",
                json!({ "title": "User preference" }),
                ToolNarrationPhase::Completed
            ),
            "Saved memory: User preference"
        );
        assert_eq!(
            narrate(
                "recall",
                json!({ "query": "preferences" }),
                ToolNarrationPhase::Started
            ),
            "Search memory: preferences"
        );
        assert_eq!(
            narrate(
                "recall",
                json!({ "id": "mem_1" }),
                ToolNarrationPhase::Started
            ),
            "Reading memory"
        );
        assert_eq!(
            narrate(
                "forget",
                json!({ "title": "Old note" }),
                ToolNarrationPhase::Completed
            ),
            "Deleted memory: Old note"
        );
    }

    #[test]
    fn renders_hook_family_with_nested_spec_id() {
        assert_eq!(
            narrate(
                "validate_hook",
                json!({ "spec": { "id": "block-git" } }),
                ToolNarrationPhase::Completed
            ),
            "Validated hook: block-git"
        );
        assert_eq!(
            narrate(
                "upsert_hook",
                json!({ "id": "block-git" }),
                ToolNarrationPhase::Completed
            ),
            "Saved hook: block-git"
        );
    }

    #[test]
    fn renders_connector_and_approval_family() {
        assert_eq!(
            narrate(
                "connect",
                json!({ "provider": "daytona" }),
                ToolNarrationPhase::Completed
            ),
            "Connected provider: daytona"
        );
        assert_eq!(
            narrate(
                "set_approval_mode",
                json!({ "mode": "protective" }),
                ToolNarrationPhase::Completed
            ),
            "Set approval mode: protective"
        );
        // The approved action is never echoed.
        assert_eq!(
            narrate(
                "record_approval",
                json!({ "action": "delete everything" }),
                ToolNarrationPhase::Completed
            ),
            "Recorded approval"
        );
    }

    #[test]
    fn generic_search_rule_covers_unseen_tools() {
        assert_eq!(
            narrate(
                "duckduckgo_search",
                json!({ "query": "rust docs" }),
                ToolNarrationPhase::Started
            ),
            "Search DuckDuckGo: rust docs"
        );
        // Unseen sibling caught by the generic *_search rule.
        assert_eq!(
            narrate(
                "wikipedia_search",
                json!({ "query": "rust" }),
                ToolNarrationPhase::Started
            ),
            "Search: rust"
        );
    }

    #[test]
    fn never_renders_secret_argument_values() {
        // A generic search whose only scannable value is a secret-named field
        // falls back to the bare verb rather than leaking the value.
        assert_eq!(
            narrate(
                "vault_search",
                json!({ "token": "super-secret" }),
                ToolNarrationPhase::Started
            ),
            "Search"
        );
    }

    #[test]
    fn narration_noun_ukrainian_uses_generic_fallback() {
        use crate::tool_types::{BuiltinTool, ToolDefinition, ToolHints};

        let tool_call = ToolCall {
            id: "call_1".to_string(),
            name: "manage_agents".to_string(),
            arguments: json!({ "operation": "create", "name": "Test Agent" }),
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

        // Ukrainian doesn't use operation_narration yet — stays in Ukrainian
        assert_eq!(
            render_tool_narration_with_locale(
                Some(&def),
                &tool_call,
                ToolNarrationPhase::Completed,
                Some("uk"),
            ),
            "Запустив Manage Agents"
        );
    }
}
