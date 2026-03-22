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

fn arg_bool(arguments: &Value, key: &str) -> Option<bool> {
    arguments.get(key).and_then(Value::as_bool)
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

fn ukrainian_phrase(
    tool_call: &ToolCall,
    fallback_name: &str,
    phase: ToolNarrationPhase,
) -> String {
    let args = &tool_call.arguments;
    match tool_call.name.as_str() {
        "bash" => {
            let command = arg_str(args, &["command"])
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
        "get_subagents" => {
            let target = arg_str(args, &["name_or_id"])
                .map(|value| format!("субагента {value}"))
                .unwrap_or_else(|| "субагентів".to_string());
            match phase {
                ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => {
                    format!("Перевіряю {target}")
                }
                ToolNarrationPhase::Completed => format!("Перевірив {target}"),
                ToolNarrationPhase::Failed => format!("Не вдалося перевірити {target}"),
            }
        }
        "message_subagent" => {
            let target = arg_str(args, &["name_or_id"])
                .map(|value| format!("повідомлення для {value}"))
                .unwrap_or_else(|| "повідомлення субагенту".to_string());
            if matches!(phase, ToolNarrationPhase::Completed)
                && arg_bool(args, "cancel").unwrap_or(false)
            {
                format!(
                    "Поставив у чергу запит на зупинку {}",
                    target.trim_start_matches("повідомлення для ")
                )
            } else {
                match phase {
                    ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => {
                        format!("Ставлю у чергу {target}")
                    }
                    ToolNarrationPhase::Completed => format!("Поставив у чергу {target}"),
                    ToolNarrationPhase::Failed => format!("Не вдалося поставити у чергу {target}"),
                }
            }
        }
        "write_todos" => match phase {
            ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => {
                "Оновлюю список задач".to_string()
            }
            ToolNarrationPhase::Completed => "Оновив список задач".to_string(),
            ToolNarrationPhase::Failed => "Не вдалося оновити список задач".to_string(),
        },
        _ => match phase {
            ToolNarrationPhase::Started | ToolNarrationPhase::Waiting => {
                format!("Запускаю {fallback_name}")
            }
            ToolNarrationPhase::Completed => format!("Запустив {fallback_name}"),
            ToolNarrationPhase::Failed => format!("Не вдалося запустити {fallback_name}"),
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
        return ukrainian_phrase(tool_call, &fallback_name, phase);
    }

    match tool_call.name.as_str() {
        "bash" => {
            let command = arg_str(args, &["command"])
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
        "get_subagents" => {
            let target = arg_str(args, &["name_or_id"])
                .map(|value| format!("subagent {value}"))
                .unwrap_or_else(|| "subagent status".to_string());
            generic_phrase(
                "Checking",
                "Checked",
                "Failed to check",
                Some(target),
                phase,
            )
        }
        "message_subagent" => {
            let target = arg_str(args, &["name_or_id"])
                .map(|value| format!("message for {value}"))
                .unwrap_or_else(|| "subagent message".to_string());
            if matches!(phase, ToolNarrationPhase::Completed)
                && arg_bool(args, "cancel").unwrap_or(false)
            {
                format!(
                    "Queued stop request for {}",
                    target.trim_start_matches("message for ")
                )
            } else {
                generic_phrase("Queueing", "Queued", "Failed to queue", Some(target), phase)
            }
        }
        "write_todos" => generic_phrase(
            "Updating",
            "Updated",
            "Failed to update",
            Some("task list".to_string()),
            phase,
        ),
        "browserless_open_browser" => {
            let target = arg_str(args, &["url"]).map(|url| truncate(url, 48));
            generic_phrase(
                "Opening browser",
                "Opened browser",
                "Failed to open browser",
                target,
                phase,
            )
        }
        "browserless_close_browser" => generic_phrase(
            "Closing browser",
            "Closed browser",
            "Failed to close browser",
            None,
            phase,
        ),
        "browserless_navigate" => {
            let target = arg_str(args, &["url"]).map(|url| truncate(url, 48));
            generic_phrase(
                "Navigating to",
                "Navigated to",
                "Failed to navigate to",
                target,
                phase,
            )
        }
        "browserless_screenshot" => generic_phrase(
            "Taking screenshot",
            "Took screenshot",
            "Failed to take screenshot",
            None,
            phase,
        ),
        "browserless_content" => generic_phrase(
            "Reading page content",
            "Read page content",
            "Failed to read page content",
            None,
            phase,
        ),
        "browserless_scrape" => generic_phrase(
            "Scraping page",
            "Scraped page",
            "Failed to scrape page",
            None,
            phase,
        ),
        "browserless_interact" => generic_phrase(
            "Interacting with page",
            "Interacted with page",
            "Failed to interact with page",
            None,
            phase,
        ),
        _ => generic_phrase(
            "Running",
            "Ran",
            "Failed to run",
            Some(fallback_name),
            phase,
        ),
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
}
