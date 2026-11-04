use super::*;
use crate::builtins::tool_types::{
    BuiltinTool, DeferrablePolicy, ToolHints, ToolPolicy, ToolResult,
};
use crate::builtins::typed_id::SessionId;

fn call(name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: format!("call-{name}"),
        name: name.to_string(),
        arguments,
    }
}

fn tool_def(name: &str) -> ToolDefinition {
    ToolDefinition::Builtin(BuiltinTool {
        name: name.to_string(),
        display_name: None,
        description: "test".to_string(),
        parameters: json!({ "type": "object" }),
        policy: ToolPolicy::Auto,
        category: None,
        deferrable: DeferrablePolicy::Never,
        hints: ToolHints::default(),
        full_parameters: None,
    })
}

fn result() -> ToolResult {
    ToolResult {
        tool_call_id: "call".to_string(),
        result: Some(json!({ "ok": true })),
        ..Default::default()
    }
}

fn result_value(value: Value) -> ToolResult {
    ToolResult {
        result: Some(value),
        ..result()
    }
}

#[test]
fn batch_reads_are_bounded_exploration_with_stable_signatures() {
    let read = call(
        "read_many_files",
        json!({"paths": ["/workspace/a", "/workspace/b"]}),
    );

    assert_eq!(classify_tool_call(&read), ToolClass::Exploration);
    assert_eq!(
        exploration_signature(&read).as_deref(),
        Some("read_many_files:{\"paths\":[\"/workspace/a\",\"/workspace/b\"]}")
    );
}

#[tokio::test]
async fn unchanged_large_read_returns_compact_marker_and_mutation_invalidates_it() {
    let state = Arc::new(Mutex::new(ProgressGuardState::default()));
    let hook = ProgressGuardHook { state };
    let context = ToolContext::new(SessionId::new());
    let read = call("read_file", json!({ "path": "/src/lib.rs" }));
    let payload = json!({ "content": "discovery evidence\n".repeat(400) });

    let mut first = result_value(payload.clone());
    hook.after_exec(&read, &tool_def("read_file"), &mut first, &context)
        .await;
    let first_bytes = serde_json::to_vec(first.result.as_ref().unwrap())
        .unwrap()
        .len();

    let mut repeated = result_value(payload.clone());
    hook.after_exec(&read, &tool_def("read_file"), &mut repeated, &context)
        .await;
    let repeated_value = repeated.result.as_ref().unwrap();
    let repeated_bytes = serde_json::to_vec(repeated_value).unwrap().len();
    assert_eq!(repeated_value["unchanged_since_last_read"], true);
    assert!(
        repeated_value["progress_guard_warning"]
            .as_str()
            .is_some_and(|warning| warning.contains("earlier full result"))
    );
    assert!(
        repeated_bytes * 10 < first_bytes,
        "compact unchanged marker should materially cut context bytes: {repeated_bytes} vs {first_bytes}"
    );

    let mut write = result();
    hook.after_exec(
        &call(
            "write_file",
            json!({ "path": "/src/lib.rs", "content": "changed" }),
        ),
        &tool_def("write_file"),
        &mut write,
        &context,
    )
    .await;
    let mut after_mutation = result_value(payload);
    hook.after_exec(&read, &tool_def("read_file"), &mut after_mutation, &context)
        .await;
    assert!(after_mutation.result.unwrap().get("content").is_some());
}

#[tokio::test]
async fn reuse_requires_the_same_target_and_same_result() {
    let state = Arc::new(Mutex::new(ProgressGuardState::default()));
    let hook = ProgressGuardHook { state };
    let context = ToolContext::new(SessionId::new());
    let large = "x".repeat(2_000);

    let cases = [
        ("/a.rs", large.clone()),
        ("/b.rs", large.clone()),
        ("/a.rs", format!("{large} changed")),
    ];
    for (path, content) in cases {
        let mut observed = result_value(json!({ "content": content }));
        hook.after_exec(
            &call("read_file", json!({ "path": path })),
            &tool_def("read_file"),
            &mut observed,
            &context,
        )
        .await;
        assert!(
            observed
                .result
                .as_ref()
                .and_then(|value| value.get("unchanged_since_last_read"))
                .is_none(),
            "different targets or changed bytes are not reusable"
        );
    }
}

#[test]
fn classify_bash_command_distinguishes_status_and_validation() {
    assert_eq!(
        classify_bash_command("git status --short --branch"),
        ToolClass::Status("git status --short --branch".to_string())
    );
    assert_eq!(
        classify_bash_command("cargo test --all-features"),
        ToolClass::Validation("cargo test --all-features".to_string())
    );
    assert_eq!(
        classify_bash_command("cargo check --all-features"),
        ToolClass::Validation("cargo check --all-features".to_string())
    );
    assert_eq!(
        classify_bash_command("cargo update -p everruns-core --precise 0.17.7"),
        ToolClass::Mutation
    );
    assert_eq!(
        classify_bash_command("rg progress_guard"),
        ToolClass::Exploration
    );
}

#[test]
fn classify_bash_command_detects_external_event_waits() {
    assert_eq!(classify_bash_command("gh pr checks 42"), ToolClass::Waiting);
    assert_eq!(
        classify_bash_command("gh run list --branch main --limit 5"),
        ToolClass::Waiting
    );
    assert_eq!(classify_bash_command("sleep 120"), ToolClass::Waiting);
    assert_eq!(
        classify_bash_command("sleep 30 && gh pr checks 42"),
        ToolClass::Waiting
    );
    // A sleep in front of real work classifies as the work itself.
    assert_eq!(
        classify_bash_command("sleep 5 && cargo test --all-features"),
        ToolClass::Validation("cargo test --all-features".to_string())
    );
    // `sleepwalk` must not parse as a sleep prefix.
    assert_eq!(classify_bash_command("sleepwalk"), ToolClass::Other);
}

#[tokio::test]
async fn hook_warns_after_long_exploration_without_progress() {
    let state = Arc::new(Mutex::new(ProgressGuardState::default()));
    let hook = ProgressGuardHook { state };
    let context = ToolContext::new(SessionId::new());
    let mut last = result();

    for _ in 0..EXPLORATION_WITHOUT_PROGRESS_THRESHOLD {
        last = result();
        hook.after_exec(
            &call("read_file", json!({ "path": "/src/lib.rs" })),
            &tool_def("read_file"),
            &mut last,
            &context,
        )
        .await;
    }

    assert!(
        last.result
            .as_ref()
            .and_then(|value| value.get("progress_guard_warning"))
            .and_then(Value::as_str)
            .is_some_and(|warning| warning.contains("investigation tools"))
    );
}

#[tokio::test]
async fn hook_escalates_to_checkpoint_after_more_exploration() {
    let state = Arc::new(Mutex::new(ProgressGuardState::default()));
    let hook = ProgressGuardHook { state };
    let context = ToolContext::new(SessionId::new());
    let mut last = result();

    for i in 0..CHECKPOINT_WITHOUT_PROGRESS_THRESHOLD {
        last = result();
        hook.after_exec(
            &call("read_file", json!({ "path": format!("/src/{i}.rs") })),
            &tool_def("read_file"),
            &mut last,
            &context,
        )
        .await;
    }

    assert!(
        last.result
            .as_ref()
            .and_then(|value| value.get("progress_guard_warning"))
            .and_then(Value::as_str)
            .is_some_and(|warning| warning.contains("checkpoint required"))
    );
}

#[tokio::test]
async fn hook_keeps_warning_after_checkpoint_until_progress() {
    let state = Arc::new(Mutex::new(ProgressGuardState::default()));
    let hook = ProgressGuardHook { state };
    let context = ToolContext::new(SessionId::new());
    let mut last = result();

    for i in 0..=CHECKPOINT_WITHOUT_PROGRESS_THRESHOLD {
        last = result();
        hook.after_exec(
            &call("grep_files", json!({ "pattern": format!("needle{i}") })),
            &tool_def("grep_files"),
            &mut last,
            &context,
        )
        .await;
    }

    assert!(
        last.result
            .as_ref()
            .and_then(|value| value.get("progress_guard_warning"))
            .and_then(Value::as_str)
            .is_some_and(|warning| warning.contains("checkpoint required"))
    );
}

#[tokio::test]
async fn mutation_resets_exploration_warning_counter() {
    let state = Arc::new(Mutex::new(ProgressGuardState::default()));
    let hook = ProgressGuardHook { state };
    let context = ToolContext::new(SessionId::new());
    let mut last = result();

    for _ in 0..(EXPLORATION_WITHOUT_PROGRESS_THRESHOLD - 1) {
        hook.after_exec(
            &call("read_file", json!({ "path": "/src/lib.rs" })),
            &tool_def("read_file"),
            &mut result(),
            &context,
        )
        .await;
    }
    hook.after_exec(
        &call("edit_file", json!({ "path": "/src/lib.rs" })),
        &tool_def("edit_file"),
        &mut result(),
        &context,
    )
    .await;
    for _ in 0..(EXPLORATION_WITHOUT_PROGRESS_THRESHOLD - 1) {
        last = result();
        hook.after_exec(
            &call("read_file", json!({ "path": "/src/lib.rs" })),
            &tool_def("read_file"),
            &mut last,
            &context,
        )
        .await;
    }

    assert!(
        last.result
            .as_ref()
            .and_then(|value| value.get("progress_guard_warning"))
            .is_none()
    );
}

#[tokio::test]
async fn validation_resets_checkpoint_warning_counter() {
    let state = Arc::new(Mutex::new(ProgressGuardState::default()));
    let hook = ProgressGuardHook { state };
    let context = ToolContext::new(SessionId::new());
    let mut last = result();

    for i in 0..CHECKPOINT_WITHOUT_PROGRESS_THRESHOLD {
        hook.after_exec(
            &call("read_file", json!({ "path": format!("/src/{i}.rs") })),
            &tool_def("read_file"),
            &mut result(),
            &context,
        )
        .await;
    }
    hook.after_exec(
        &call("bash", json!({ "commands": "cargo test --all-features" })),
        &tool_def("bash"),
        &mut result(),
        &context,
    )
    .await;
    for i in 0..(EXPLORATION_WITHOUT_PROGRESS_THRESHOLD - 1) {
        last = result();
        hook.after_exec(
            &call("read_file", json!({ "path": format!("/src/after/{i}.rs") })),
            &tool_def("read_file"),
            &mut last,
            &context,
        )
        .await;
    }

    assert!(
        last.result
            .as_ref()
            .and_then(|value| value.get("progress_guard_warning"))
            .is_none()
    );
}

#[tokio::test]
async fn lockfile_updates_do_not_hide_a_manifest_state_cycle() {
    let state = Arc::new(Mutex::new(ProgressGuardState::default()));
    let hook = ProgressGuardHook { state };
    let context = ToolContext::new(SessionId::new());
    let transitions = [("A", "B"), ("B", "C"), ("C", "A")];

    for (index, (previous, current)) in transitions.into_iter().enumerate() {
        let mut edit = result_value(json!({
            "path": "/workspace/Cargo.toml",
            "previous_content_hash": previous,
            "content_hash": current,
        }));
        hook.after_exec(
            &call("edit_file", json!({ "path": "/workspace/Cargo.toml" })),
            &tool_def("edit_file"),
            &mut edit,
            &context,
        )
        .await;

        let warning = edit
            .result
            .as_ref()
            .and_then(|value| value.get("progress_guard_warning"))
            .and_then(Value::as_str);
        if index < 2 {
            assert!(warning.is_none());
        } else {
            assert!(warning.is_some_and(|text| text.contains("workspace state")));
        }

        let mut update = result();
        hook.after_exec(
            &call(
                "bash",
                json!({ "commands": "cargo update -p everruns-core --precise 0.17.7" }),
            ),
            &tool_def("bash"),
            &mut update,
            &context,
        )
        .await;
    }
}

#[tokio::test]
async fn lockfile_update_makes_repeated_validation_fresh() {
    let state = Arc::new(Mutex::new(ProgressGuardState::default()));
    let hook = ProgressGuardHook { state };
    let context = ToolContext::new(SessionId::new());
    let validation = call("bash", json!({ "commands": "cargo check" }));

    hook.after_exec(&validation, &tool_def("bash"), &mut result(), &context)
        .await;
    hook.after_exec(
        &call(
            "bash",
            json!({ "commands": "cargo update -p everruns-core --precise 0.17.7" }),
        ),
        &tool_def("bash"),
        &mut result(),
        &context,
    )
    .await;

    let mut after_update = result();
    hook.after_exec(&validation, &tool_def("bash"), &mut after_update, &context)
        .await;
    assert!(
        after_update
            .result
            .as_ref()
            .and_then(|value| value.get("progress_guard_warning"))
            .is_none(),
        "validation after a lockfile mutation has new workspace evidence"
    );

    let mut unchanged_again = result();
    hook.after_exec(
        &validation,
        &tool_def("bash"),
        &mut unchanged_again,
        &context,
    )
    .await;
    assert!(
        unchanged_again
            .result
            .as_ref()
            .and_then(|value| value.get("progress_guard_warning"))
            .is_some(),
        "a second validation without another mutation is redundant"
    );
}

#[tokio::test]
async fn repeated_validation_on_unchanged_state_warns() {
    let state = Arc::new(Mutex::new(ProgressGuardState::default()));
    let hook = ProgressGuardHook { state };
    let context = ToolContext::new(SessionId::new());
    let validation = call("bash", json!({ "commands": "cargo test" }));

    let mut first = result();
    hook.after_exec(&validation, &tool_def("bash"), &mut first, &context)
        .await;
    assert!(
        first
            .result
            .as_ref()
            .and_then(|value| value.get("progress_guard_warning"))
            .is_none()
    );

    let mut repeated = result();
    hook.after_exec(&validation, &tool_def("bash"), &mut repeated, &context)
        .await;
    assert!(
        repeated
            .result
            .as_ref()
            .and_then(|value| value.get("progress_guard_warning"))
            .and_then(Value::as_str)
            .is_some_and(|text| text.contains("unchanged workspace state"))
    );

    let mut edit = result_value(json!({
        "path": "/workspace/src/lib.rs",
        "previous_content_hash": "A",
        "content_hash": "B",
    }));
    hook.after_exec(
        &call("edit_file", json!({ "path": "/workspace/src/lib.rs" })),
        &tool_def("edit_file"),
        &mut edit,
        &context,
    )
    .await;

    let mut after_progress = result();
    hook.after_exec(
        &validation,
        &tool_def("bash"),
        &mut after_progress,
        &context,
    )
    .await;
    assert!(
        after_progress
            .result
            .as_ref()
            .and_then(|value| value.get("progress_guard_warning"))
            .is_none(),
        "the same validation is useful after the workspace changes"
    );
}

#[tokio::test]
async fn semantic_progress_has_no_fixed_session_iteration_limit() {
    let state = Arc::new(Mutex::new(ProgressGuardState::default()));
    let hook = ProgressGuardHook { state };
    let context = ToolContext::new(SessionId::new());

    for index in 0..256 {
        let mut out = result_value(json!({
            "path": "/workspace/src/lib.rs",
            "previous_content_hash": format!("state-{index}"),
            "content_hash": format!("state-{}", index + 1),
        }));
        hook.after_exec(
            &call("edit_file", json!({ "path": "/workspace/src/lib.rs" })),
            &tool_def("edit_file"),
            &mut out,
            &context,
        )
        .await;
        assert!(
            out.result
                .as_ref()
                .and_then(|value| value.get("progress_guard_warning"))
                .is_none(),
            "each new state remains progress after iteration {index}"
        );
    }
}

#[tokio::test]
async fn repeated_exploration_target_warns() {
    let state = Arc::new(Mutex::new(ProgressGuardState::default()));
    let hook = ProgressGuardHook { state };
    let context = ToolContext::new(SessionId::new());
    let mut last = result();

    for _ in 0..REPEATED_EXPLORATION_THRESHOLD {
        last = result();
        hook.after_exec(
            &call("read_file", json!({ "path": "/src/lib.rs", "offset": 10 })),
            &tool_def("read_file"),
            &mut last,
            &context,
        )
        .await;
    }

    assert!(
        last.result
            .as_ref()
            .and_then(|value| value.get("progress_guard_warning"))
            .and_then(Value::as_str)
            .is_some_and(|warning| warning.contains("same investigation target"))
    );
}

#[tokio::test]
async fn three_zero_evidence_searches_warn_even_when_queries_differ() {
    let state = Arc::new(Mutex::new(ProgressGuardState::default()));
    let hook = ProgressGuardHook { state };
    let context = ToolContext::new(SessionId::new());
    let mut last = result();

    for pattern in ["history", "ground", "session"] {
        last = result_value(json!({ "ok": true, "count": 0, "matches": [] }));
        hook.after_exec(
            &call("grep_files", json!({ "pattern": pattern })),
            &tool_def("grep_files"),
            &mut last,
            &context,
        )
        .await;
    }

    assert!(
        last.result
            .as_ref()
            .and_then(|value| value.get("progress_guard_warning"))
            .and_then(Value::as_str)
            .is_some_and(|warning| warning.contains("zero matches"))
    );
}

#[tokio::test]
async fn positive_search_evidence_resets_zero_result_streak() {
    let state = Arc::new(Mutex::new(ProgressGuardState::default()));
    let hook = ProgressGuardHook { state };
    let context = ToolContext::new(SessionId::new());

    for count in [0, 0, 1, 0, 0] {
        let mut out = result_value(json!({ "ok": true, "count": count }));
        hook.after_exec(
            &call("repo_map", json!({ "query": format!("query-{count}") })),
            &tool_def("repo_map"),
            &mut out,
            &context,
        )
        .await;
        assert!(
            out.result
                .as_ref()
                .and_then(|value| value.get("progress_guard_warning"))
                .is_none(),
            "a positive result should break the zero-evidence streak"
        );
    }
}

#[tokio::test]
async fn repeated_truncated_exploration_warns_to_narrow_scope() {
    let state = Arc::new(Mutex::new(ProgressGuardState::default()));
    let hook = ProgressGuardHook { state };
    let context = ToolContext::new(SessionId::new());
    let mut last = result();

    for query in ["runtime", "capability"] {
        last = result_value(json!({ "ok": true, "count": 50, "truncated": true }));
        hook.after_exec(
            &call("repo_map", json!({ "query": query })),
            &tool_def("repo_map"),
            &mut last,
            &context,
        )
        .await;
    }

    assert!(
        last.result
            .as_ref()
            .and_then(|value| value.get("progress_guard_warning"))
            .and_then(Value::as_str)
            .is_some_and(|warning| warning.contains("truncated"))
    );
}

#[tokio::test]
async fn original_session_pattern_gets_interrupted_before_runaway_reads() {
    let state = Arc::new(Mutex::new(ProgressGuardState::default()));
    let hook = ProgressGuardHook { state };
    let context = ToolContext::new(SessionId::new());
    let mut warnings = Vec::new();

    let observe = |tool_call: ToolCall| {
        let hook = &hook;
        let context = &context;
        async move {
            let mut out = result();
            hook.after_exec(&tool_call, &tool_def(&tool_call.name), &mut out, context)
                .await;
            out.result
                .as_ref()
                .and_then(|value| value.get("progress_guard_warning"))
                .and_then(Value::as_str)
                .map(str::to_string)
        }
    };

    // Same shape as the failed investigation: broad searches, then many
    // repeated reads of the callback-adjacent files, with no edit/test.
    let broad_searches = [
        "scheduled|schedule|signal_on_completion|callback|background",
        "spawn_background|signal_on_completion|scheduled_at",
        "cron|completion|task|background_run|Background|Task",
        "drain_finished_for_wake|wake_prompt|BackgroundRegistry",
        "SessionScheduleStore|schedule_store|create_schedule",
        "maybe_wake_for_background|TaskRegistryEvent",
    ];
    for pattern in broad_searches {
        if let Some(warning) = observe(call(
            "grep_files",
            json!({ "pattern": pattern, "path_pattern": "src/**/*.rs" }),
        ))
        .await
        {
            warnings.push(warning);
        }
    }
    for offset in [180, 388, 633, 940, 1370, 1540, 180, 388, 633] {
        if let Some(warning) = observe(call(
            "read_file",
            json!({ "path": "/repo/src/capabilities/background.rs", "offset": offset }),
        ))
        .await
        {
            warnings.push(warning);
        }
    }
    for offset in [1000, 1020, 1010, 1028, 1000, 1020, 1010, 1028, 1000] {
        if let Some(warning) = observe(call(
            "read_file",
            json!({ "path": "/repo/src/app/mod.rs", "offset": offset }),
        ))
        .await
        {
            warnings.push(warning);
        }
    }
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("investigation tools")),
        "first threshold should warn before the session keeps circling: {warnings:?}"
    );

    for i in 0..24 {
        if let Some(warning) = observe(call(
            "read_file",
            json!({ "path": "/repo/src/runtime.rs", "offset": 2100 + i }),
        ))
        .await
        {
            warnings.push(warning);
        }
    }
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("checkpoint required")),
        "checkpoint escalation should trigger by 48 read/search calls: {warnings:?}"
    );

    for _ in 0..REPEATED_EXPLORATION_THRESHOLD {
        if let Some(warning) = observe(call(
            "read_file",
            json!({ "path": "/repo/src/app/mod.rs", "offset": 1000 }),
        ))
        .await
        {
            warnings.push(warning);
        }
    }
    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("same investigation target")),
        "semantic repetition should catch rereading the same range: {warnings:?}"
    );
}

#[tokio::test]
async fn repeated_ci_polling_warns_toward_spawn_background() {
    let state = Arc::new(Mutex::new(ProgressGuardState::default()));
    let hook = ProgressGuardHook { state };
    let context = ToolContext::new(SessionId::new());
    let mut last = result();

    // The classic poll loop: delay-then-check, over and over. Different
    // probe commands still count — polling rarely repeats verbatim.
    let polls = [
        "gh pr checks 42",
        "sleep 30 && gh pr checks 42",
        "gh run list --limit 1",
        "gh pr view 42",
    ];
    for command in polls {
        last = result();
        hook.after_exec(
            &call("bash", json!({ "commands": command })),
            &tool_def("bash"),
            &mut last,
            &context,
        )
        .await;
    }

    assert!(
        last.result
            .as_ref()
            .and_then(|value| value.get("progress_guard_warning"))
            .and_then(Value::as_str)
            .is_some_and(|warning| warning.contains("spawn_background"))
    );
}

#[tokio::test]
async fn semantic_polling_cycle_warns_across_heterogeneous_tools() {
    let state = Arc::new(Mutex::new(ProgressGuardState::default()));
    let hook = ProgressGuardHook { state };
    let context = ToolContext::new(SessionId::new());
    let cycle = [
        call("bash", json!({ "commands": "gh pr checks 42" })),
        call("read_file", json!({ "path": "/tmp/synthetic-ci-note" })),
        call("get_task", json!({ "task_id": "task_ci" })),
        call("bash", json!({ "commands": "gh run view 123" })),
    ];
    let mut warnings = Vec::new();

    for tool_call in cycle.into_iter().cycle().take(8) {
        let mut output = result();
        hook.after_exec(
            &tool_call,
            &tool_def(&tool_call.name),
            &mut output,
            &context,
        )
        .await;
        if let Some(warning) = output
            .result
            .as_ref()
            .and_then(|value| value.get("progress_guard_warning"))
            .and_then(Value::as_str)
        {
            warnings.push(warning.to_string());
        }
    }

    assert!(
        warnings
            .iter()
            .any(|warning| warning.contains("spawn_background")),
        "a semantic polling cycle must be caught even when unrelated reads and task probes separate external checks: {warnings:?}"
    );
}

#[tokio::test]
async fn one_off_task_and_ci_status_checks_do_not_warn() {
    let state = Arc::new(Mutex::new(ProgressGuardState::default()));
    let hook = ProgressGuardHook { state };
    let context = ToolContext::new(SessionId::new());

    for tool_call in [
        call("list_tasks", json!({})),
        call("get_task", json!({ "task_id": "task_once" })),
        call("bash", json!({ "commands": "gh pr checks 42" })),
    ] {
        let mut output = result();
        hook.after_exec(
            &tool_call,
            &tool_def(&tool_call.name),
            &mut output,
            &context,
        )
        .await;
        assert!(
            output
                .result
                .as_ref()
                .and_then(|value| value.get("progress_guard_warning"))
                .is_none(),
            "a one-off status check is legitimate"
        );
    }
}

#[tokio::test]
async fn interleaved_work_resets_waiting_window() {
    let state = Arc::new(Mutex::new(ProgressGuardState::default()));
    let hook = ProgressGuardHook { state };
    let context = ToolContext::new(SessionId::new());

    // Check CI, fix something, check again — legitimate, because real
    // progress clears the semantic window.
    for _ in 0..(WAITING_WINDOW_THRESHOLD * 2) {
        let mut check = result();
        hook.after_exec(
            &call("bash", json!({ "commands": "gh pr checks 42" })),
            &tool_def("bash"),
            &mut check,
            &context,
        )
        .await;
        assert!(
            check
                .result
                .as_ref()
                .and_then(|value| value.get("progress_guard_warning"))
                .is_none()
        );
        hook.after_exec(
            &call("edit_file", json!({ "path": "/src/lib.rs" })),
            &tool_def("edit_file"),
            &mut result(),
            &context,
        )
        .await;
    }
}

#[tokio::test]
async fn repeated_git_status_warns() {
    let state = Arc::new(Mutex::new(ProgressGuardState::default()));
    let hook = ProgressGuardHook { state };
    let context = ToolContext::new(SessionId::new());
    let mut last = result();

    for _ in 0..REPEATED_STATUS_THRESHOLD {
        last = result();
        hook.after_exec(
            &call("bash", json!({ "commands": "git status --short" })),
            &tool_def("bash"),
            &mut last,
            &context,
        )
        .await;
    }

    assert!(
        last.result
            .as_ref()
            .and_then(|value| value.get("progress_guard_warning"))
            .and_then(Value::as_str)
            .is_some_and(|warning| warning.contains("repeated git status"))
    );
}
