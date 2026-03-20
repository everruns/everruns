use serde_json::Value;

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

fn display_name(tool_def: Option<&ToolDefinition>, tool_call: &ToolCall) -> String {
    tool_def
        .and_then(|def| def.display_name())
        .map(ToOwned::to_owned)
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

fn location_phrase(arguments: &Value) -> String {
    arg_str(arguments, &["path", "directory", "working_dir"])
        .map(|value| {
            if value == "." || value == "/workspace" {
                "current directory".to_string()
            } else {
                value.to_string()
            }
        })
        .unwrap_or_else(|| "current directory".to_string())
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

pub fn render_tool_narration(
    tool_def: Option<&ToolDefinition>,
    tool_call: &ToolCall,
    phase: ToolNarrationPhase,
) -> String {
    let args = &tool_call.arguments;
    let fallback_name = display_name(tool_def, tool_call);

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
            Some(location_phrase(args)),
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
        _ => generic_phrase(
            "Running",
            "Ran",
            "Failed to run",
            Some(fallback_name),
            phase,
        ),
    }
}

fn join_phrases(phrases: &[String], total_count: usize) -> String {
    match phrases {
        [] => "Working".to_string(),
        [only] => only.clone(),
        [first, second] => format!("{first} and {second}"),
        [first, second, ..] => {
            format!(
                "{first}, {second}, and {} more actions",
                total_count.saturating_sub(2)
            )
        }
    }
}

pub fn render_group_headline(
    tool_calls: &[ToolCall],
    tool_defs: &[ToolDefinition],
    phase: ToolNarrationPhase,
) -> Option<String> {
    if tool_calls.is_empty() {
        return None;
    }

    let tool_map: std::collections::HashMap<&str, &ToolDefinition> =
        tool_defs.iter().map(|def| (def.name(), def)).collect();

    let phrases = tool_calls
        .iter()
        .map(|tool_call| {
            render_tool_narration(
                tool_map.get(tool_call.name.as_str()).copied(),
                tool_call,
                phase,
            )
        })
        .take(3)
        .collect::<Vec<_>>();

    Some(join_phrases(&phrases, tool_calls.len()))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{ToolNarrationPhase, render_group_headline, render_tool_narration};
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
}
