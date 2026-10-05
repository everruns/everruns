use serde_json::json;

use super::*;
use crate::tool_types::ToolCall;

#[test]
fn list_directory_helper_without_store_echoes_legacy_alias() {
    let a = json!({ "path": "/workspace/crates" });
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
fn list_directory_delegates_to_the_correct_store_path_contract() {
    use crate::error::Result;
    use crate::runtime::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
    use crate::typed_id::SessionId;
    use async_trait::async_trait;

    struct HostBackedStore {
        mount_resolver: bool,
    }

    #[async_trait]
    impl SessionFileSystem for HostBackedStore {
        fn display_path(&self, path: &str) -> String {
            assert!(
                !self.mount_resolver,
                "mount resolvers must use resolve_path"
            );
            format!("display::{path}")
        }

        fn resolve_path(&self, path: &str) -> String {
            assert!(self.mount_resolver, "ordinary stores must use display_path");
            format!("resolve::{path}")
        }

        fn is_mount_resolver(&self) -> bool {
            self.mount_resolver
        }

        async fn read_file(&self, _: SessionId, _: &str) -> Result<Option<SessionFile>> {
            Ok(None)
        }
        async fn write_file(&self, _: SessionId, _: &str, _: &str, _: &str) -> Result<SessionFile> {
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

    for mount_resolver in [false, true] {
        let store = HostBackedStore { mount_resolver };
        let ctx = ToolNarrationContext::new(Some(&store));
        let prefix = if mount_resolver { "resolve" } else { "display" };
        for (arguments, expected_path) in [
            (json!({"path":" /workspace/crates "}), "/workspace/crates"),
            (json!({"path":"crates"}), "crates"),
            (json!({"path":"/repo/crates"}), "/repo/crates"),
            (json!({}), "/workspace"),
            (json!({"path":" "}), "/workspace"),
        ] {
            for (phase, verb) in [
                (ToolNarrationPhase::Started, "Listing files in"),
                (ToolNarrationPhase::Waiting, "Listing files in"),
                (ToolNarrationPhase::Completed, "Listed files in"),
                (ToolNarrationPhase::Failed, "Failed to list files in"),
            ] {
                assert_eq!(
                    narrate_list_directory(&arguments, phase, None, ctx),
                    format!("{verb} {prefix}::{expected_path}")
                );
            }
        }
    }
}

#[test]
fn shell_exec_helper_en_and_uk() {
    let a = json!({ "command": "cargo test" });
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
    let a = json!({ "path": "/workspace/AGENTS.md" });
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
    let a = json!({ "path": "/workspace/src/main.rs" });
    assert_eq!(
        narrate_edit_file(&a, ToolNarrationPhase::Completed, None),
        "Edited main.rs"
    );
}

#[test]
fn web_fetch_helper_strips_scheme_and_query() {
    let a = json!({ "url": "https://example.com/page?token=abc#frag" });
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
    let a = json!({
        "url": "https://example.com/file?token=abc",
        "save_to_file": "/downloads/file"
    });
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

    let blank = json!({
        "url": "https://example.com/file",
        "save_to_file": "   "
    });
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
        let a = json!({ "url": raw });
        let started = narrate_web_fetch(&a, ToolNarrationPhase::Started, None);
        let completed = narrate_web_fetch(&a, ToolNarrationPhase::Completed, None);
        assert_eq!(started, "Fetching URL", "input: {raw}");
        assert_eq!(completed, "Fetched URL", "input: {raw}");
    }
}

#[test]
fn secret_key_filter_rejects_every_candidate_before_using_safe_fallback() {
    for key in [
        "token",
        "ACCESS_TOKEN",
        "api_key",
        "APIKEY",
        "user_password",
        "clientSecret",
        "Authorization",
    ] {
        let arguments = json!({key:"SENSITIVE_VALUE","query":" public query "});
        assert_eq!(safe_arg_str(&arguments, &[key]), None, "{key}");
        assert_eq!(
            safe_arg_str(&arguments, &[key, "query"]),
            Some("public query"),
            "{key}"
        );
        assert_eq!(
            narrate_provider_search(&arguments, ToolNarrationPhase::Started, None),
            "Searching: public query"
        );
    }
    assert_eq!(
        narrate_provider_search(
            &json!({"token":"SENSITIVE_VALUE"}),
            ToolNarrationPhase::Started,
            None
        ),
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
fn skill_helper_dispatches_every_family_member_and_rejects_other_names() {
    for (name, expected) in [
        ("activate_skill", Some("Activated skill: ship")),
        ("read_skill", Some("Read skill: ship")),
        ("write_skill", Some("Wrote skill: ship")),
        ("list_skills", Some("Listed skills")),
        ("not_a_skill", None),
    ] {
        assert_eq!(
            narrate_skill(
                name,
                &json!({"name":"ship"}),
                ToolNarrationPhase::Completed,
                None
            )
            .as_deref(),
            expected,
            "{name}"
        );
    }
    assert_eq!(
        narrate_skill(
            "write_skill",
            &json!({"name":"pdf"}),
            ToolNarrationPhase::Completed,
            None
        )
        .as_deref(),
        Some("Wrote skill: pdf")
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
    let a = json!({ "title": "Improve tool narration" });
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
fn write_session_title_truncates_exact_characters_and_drops_blank_detail() {
    for count in [47, 48, 49] {
        let title = "é".repeat(count);
        let expected_detail = if count <= 48 {
            title.clone()
        } else {
            format!("{}...", "é".repeat(48))
        };
        assert_eq!(
            narrate_write_session_title(&json!({"title":title}), ToolNarrationPhase::Started, None),
            format!("Updating session title: {expected_detail}")
        );
    }
    for arguments in [json!({}), json!({"title":"  "}), json!({"title":null})] {
        assert_eq!(
            narrate_write_session_title(&arguments, ToolNarrationPhase::Started, None),
            "Updating session title"
        );
    }
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
    let a = json!({ "description": "Run nightly backup" });
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
    let a = json!({ "task_id": "task_abc" });
    assert_eq!(
        narrate_session_task("get_task", &a, ToolNarrationPhase::Completed, None),
        Some("Read task: task_abc".to_string())
    );
    assert_eq!(
        narrate_session_task("list_tasks", &json!({}), ToolNarrationPhase::Started, None),
        Some("Listing tasks".to_string())
    );
    assert_eq!(
        narrate_session_task("message_task", &a, ToolNarrationPhase::Completed, None).as_deref(),
        Some("Messaged task: task_abc")
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
    let a = json!({ "action": "pause" });
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
    let a = json!({ "sql": "SELECT * FROM users WHERE api_token = 'SECRET42'" });
    assert_eq!(
        narrate_sql("sql_query", &a, ToolNarrationPhase::Completed, None),
        Some("Queried SQL".to_string())
    );
    assert_eq!(
        narrate_sql("sql_execute", &a, ToolNarrationPhase::Started, None),
        Some("Running SQL".to_string())
    );
    assert_eq!(
        narrate_sql("sql_schema", &a, ToolNarrationPhase::Started, None),
        Some("Reading database schema".to_string())
    );
    assert_eq!(
        narrate_sql("other", &json!({}), ToolNarrationPhase::Started, None),
        None
    );
}

#[test]
fn knowledge_and_history_search_show_queries_and_ignore_unrelated_fields() {
    let query = json!({ "query": "orders" });
    assert_eq!(
        narrate_search_knowledge(&query, ToolNarrationPhase::Completed, None),
        "Searched knowledge: orders"
    );
    assert_eq!(
        narrate_query_history(&query, ToolNarrationPhase::Completed, None),
        "Searched history: orders"
    );
    // A secret-named field is never rendered even if it is the only arg.
    let secret = json!({ "api_key": "sk-live-123" });
    assert_eq!(
        narrate_search_knowledge(&secret, ToolNarrationPhase::Started, None),
        "Searching knowledge"
    );
    assert_eq!(
        narrate_query_history(&secret, ToolNarrationPhase::Started, None),
        "Searching history"
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
fn fallback_prefers_definition_display_name_then_title_cases_unowned_tools() {
    let call = ToolCall {
        id: "call_1".into(),
        name: "mystery__tool".into(),
        arguments: json!({"token":"SENSITIVE_VALUE"}),
    };
    let definition = tool_definition("mystery__tool", Some("Friendly Tool"), None);
    assert_eq!(
        render_tool_narration(Some(&definition), &call, ToolNarrationPhase::Completed),
        "Ran Friendly Tool"
    );
    assert_eq!(
        render_tool_narration(None, &call, ToolNarrationPhase::Completed),
        "Ran Mystery Tool"
    );
    assert_eq!(
        render_tool_narration_with_locale(
            Some(&definition),
            &call,
            ToolNarrationPhase::Waiting,
            Some("uk-UA")
        ),
        "Запускаю Friendly Tool"
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
fn group_headline_counts_repeats_with_localized_boundaries() {
    for (count, locale, expected) in [
        (2, None, "Searched files twice"),
        (3, None, "Searched files 3 times"),
        (2, Some("uk"), "Шукав у файлах двічі"),
        (3, Some("uk"), "Шукав у файлах 3 рази"),
        (5, Some("uk"), "Шукав у файлах 5 разів"),
        (12, Some("uk"), "Шукав у файлах 12 разів"),
        (22, Some("uk"), "Шукав у файлах 22 рази"),
    ] {
        let repeated = if locale.is_some() {
            "Шукав у файлах"
        } else {
            "Searched files"
        };
        let actions = (0..count)
            .map(|i| group_action("grep_files", &format!("query {i}"), repeated))
            .collect::<Vec<_>>();
        assert_eq!(summarize_group_actions(&actions, locale), expected);
    }
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

#[test]
fn subagent_spawn_narration_names_the_spawned_agent() {
    let call = serde_json::json!({
        "name": "Orbit Scout",
        "instructions": "Inspect the orbit subsystem.",
        "target": { "type": "subagent" }
    });
    assert_eq!(
        narrate_subagent_spawn(&call, ToolNarrationPhase::Started, None),
        "Launching Orbit Scout subagent"
    );
    assert_eq!(
        narrate_subagent_spawn(&call, ToolNarrationPhase::Completed, None),
        "Launched Orbit Scout subagent"
    );
}

#[test]
fn subagent_spawn_narration_includes_blueprint_and_target_id() {
    let blueprint = serde_json::json!({
        "name": "Orbit Scout",
        "target": { "type": "subagent" },
        "blueprint": "github_scout"
    });
    assert_eq!(
        narrate_subagent_spawn(&blueprint, ToolNarrationPhase::Started, None),
        "Launching Orbit Scout subagent (github_scout)"
    );

    let handoff = serde_json::json!({
        "name": "Release check",
        "target": { "type": "agent", "id": "release-reviewer" }
    });
    assert_eq!(
        narrate_subagent_spawn(&handoff, ToolNarrationPhase::Started, None),
        "Launching Release check agent (release-reviewer)"
    );

    let external = serde_json::json!({
        "target": { "type": "external_a2a", "id": "partner-agent" }
    });
    assert_eq!(
        narrate_subagent_spawn(&external, ToolNarrationPhase::Failed, None),
        "Failed to launch external agent (partner-agent)"
    );
}

#[test]
fn subagent_spawn_narration_ignores_decoy_blueprint_for_configured_targets() {
    let handoff = serde_json::json!({
        "name": "Release check",
        "target": { "type": "agent", "id": "release-reviewer" },
        "blueprint": "trusted-reviewer"
    });
    assert_eq!(
        narrate_subagent_spawn(&handoff, ToolNarrationPhase::Started, None),
        "Launching Release check agent (release-reviewer)"
    );

    let external = serde_json::json!({
        "target": { "type": "external_a2a", "external_agent_id": "partner-agent" },
        "blueprint": "trusted-reviewer"
    });
    assert_eq!(
        narrate_subagent_spawn(&external, ToolNarrationPhase::Started, None),
        "Launching external agent (partner-agent)"
    );
}

#[test]
fn subagent_spawn_narration_is_localized() {
    let call = serde_json::json!({
        "name": "Orbit Scout",
        "target": { "type": "subagent" }
    });
    assert_eq!(
        narrate_subagent_spawn(&call, ToolNarrationPhase::Started, Some("uk")),
        "Запускаю субагента Orbit Scout"
    );
}
#[test]
fn argument_aliases_skip_empty_or_non_string_candidates() {
    for first in [json!(null), json!("  "), json!(12), json!([]), json!({})] {
        let arguments = json!({"commands":first,"command":"  cargo test  "});
        assert_eq!(
            arg_str(&arguments, &["commands", "command"]),
            Some("cargo test")
        );
        assert_eq!(
            safe_arg_str(&arguments, &["commands", "command"]),
            Some("cargo test")
        );
        assert_eq!(
            narrate_shell_exec(&arguments, "Shell", ToolNarrationPhase::Completed, None),
            "Ran `cargo test`"
        );
    }
    let arguments = json!({"query":"first","q":"second"});
    assert_eq!(arg_str(&arguments, &["query", "q"]), Some("first"));
    assert_eq!(safe_arg_str(&arguments, &["query", "q"]), Some("first"));
}

fn tool_definition(name: &str, display: Option<&str>, noun: Option<&str>) -> ToolDefinition {
    use crate::tool_types::{BuiltinTool, ToolHints};
    ToolDefinition::Builtin(BuiltinTool {
        name: name.into(),
        display_name: display.map(str::to_owned),
        description: String::new(),
        parameters: json!({}),
        policy: Default::default(),
        category: None,
        deferrable: Default::default(),
        hints: noun.map_or_else(ToolHints::default, |noun| {
            ToolHints::default().with_narration_noun(noun)
        }),
        full_parameters: None,
    })
}

#[test]
fn group_renderer_preserves_operation_identity_order_and_omitted_call_count() {
    let definitions = [
        tool_definition("manage", Some("Manage"), Some("agent")),
        tool_definition("other", Some("Other"), Some("agent")),
    ];
    let call = |tool: &str, operation: &str, name: &str| ToolCall {
        id: format!("call-{name}"),
        name: tool.into(),
        arguments: json!({"operation":operation,"name":name,"token":"SENSITIVE_VALUE"}),
    };
    let calls = [
        call("manage", "create", "first"),
        call("manage", "create", "second"),
        call("manage", "delete", "third"),
        call("other", "create", "fourth"),
        call("other", "create", "fifth"),
    ];
    assert_eq!(
        render_group_headline(&[], &definitions, ToolNarrationPhase::Completed),
        None
    );
    assert_eq!(
        render_group_headline(&calls[..1], &definitions, ToolNarrationPhase::Completed).as_deref(),
        Some("Created agent: first")
    );
    assert_eq!(
        render_group_headline(&calls, &definitions, ToolNarrationPhase::Completed).as_deref(),
        Some("Created agent twice, Deleted agent: third, and 2 more actions")
    );
    // Same wording from different tools must remain two distinct actions.
    assert_eq!(
        render_group_headline(
            &[calls[0].clone(), calls[3].clone()],
            &definitions,
            ToolNarrationPhase::Completed
        )
        .as_deref(),
        Some("Created agent: first and Created agent: fourth")
    );
    let original = ToolCall {
        id: "identity".into(),
        name: "manage".into(),
        arguments: json!({"action":"delete","operation":"create","name":"private","token":"SENSITIVE_VALUE"}),
    };
    let summary = tool_call_for_group_summary(&original);
    assert_eq!(summary.id, "identity");
    assert_eq!(summary.name, "manage");
    assert_eq!(
        summary.arguments,
        json!({"action":"delete","operation":"create"})
    );
    assert_eq!(original.arguments["name"], "private");
}

#[test]
fn foreign_fallback_shows_safe_resource_labels_without_dumping_bodies() {
    let call = ToolCall {
        id: "foreign".into(),
        name: "foreign_tool".into(),
        arguments: json!({"path":"/workspace/reports/status.md", "prompt":"PRIVATE", "content":"PRIVATE", "api_key":"PRIVATE"}),
    };
    let definition = tool_definition("foreign_tool", Some("External reader"), None);
    assert_eq!(
        render_tool_narration(Some(&definition), &call, ToolNarrationPhase::Completed),
        "Ran External reader: status.md"
    );
    let call = ToolCall {
        arguments: json!({"url":"https://user:secret@example.com/report?token=secret#secret"}),
        ..call
    };
    assert_eq!(
        render_tool_narration(Some(&definition), &call, ToolNarrationPhase::Failed),
        "Failed to run External reader: example.com/report"
    );
}
