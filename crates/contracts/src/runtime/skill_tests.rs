use super::*;

#[test]
fn test_parse_valid_skill_md() {
    let content = r#"---
name: pdf-processing
description: Extract text from PDF files.
---

# PDF Processing

Use pdfplumber to extract text.
"#;
    let parsed = parse_skill_md(content).unwrap();
    assert_eq!(parsed.name, "pdf-processing");
    assert_eq!(parsed.description, "Extract text from PDF files.");
    assert_eq!(
        parsed.instructions,
        "# PDF Processing\n\nUse pdfplumber to extract text.\n"
    );
    assert!(parsed.user_invocable);
    assert!(!parsed.disable_model_invocation);
    assert_eq!(parsed.argument_hint, None);
    assert_eq!(parsed.context, SkillContext::Inline);
    assert_eq!(parsed.agent, None);
    assert_eq!(parsed.model, None);
    assert_eq!(parsed.version, "1.0");
}

#[test]
fn test_parse_with_optional_fields() {
    let content = r#"---
name: data-analysis
description: Analyze datasets.
license: MIT
compatibility: Python 3.10+
metadata:
  version: "2.0"
  author: test
allowed-tools: bash python
---

Instructions here.
"#;
    let parsed = parse_skill_md(content).unwrap();
    assert_eq!(parsed.name, "data-analysis");
    assert_eq!(parsed.license.as_deref(), Some("MIT"));
    assert_eq!(parsed.compatibility.as_deref(), Some("Python 3.10+"));
    assert_eq!(parsed.version, "2.0");
    assert_eq!(parsed.allowed_tools.as_deref(), Some("bash python"));
}

#[test]
fn test_parse_missing_name() {
    let content = r#"---
description: No name here.
---

Body.
"#;
    let err = parse_skill_md(content).unwrap_err();
    assert!(err.iter().any(|e| e.contains("name: required")));
}

#[test]
fn test_parse_missing_description() {
    let content = r#"---
name: test-skill
---

Body.
"#;
    let err = parse_skill_md(content).unwrap_err();
    assert!(err.iter().any(|e| e.contains("description: required")));
}

#[test]
fn test_parse_no_frontmatter() {
    let content = "# Just markdown, no frontmatter";
    let err = parse_skill_md(content).unwrap_err();
    assert!(err.iter().any(|e| e.contains("frontmatter")));
}

#[test]
fn test_validate_name_valid() {
    assert!(validate_skill_name("pdf-processing").is_ok());
    assert!(validate_skill_name("a").is_ok());
    assert!(validate_skill_name("my-skill-123").is_ok());
}

#[test]
fn test_validate_name_invalid() {
    assert!(validate_skill_name("").is_err());
    assert!(validate_skill_name("-leading").is_err());
    assert!(validate_skill_name("trailing-").is_err());
    assert!(validate_skill_name("double--hyphen").is_err());
    assert!(validate_skill_name("UPPERCASE").is_err());
    assert!(validate_skill_name("has spaces").is_err());
    assert!(validate_skill_name("has_underscores").is_err());
}

#[test]
fn test_validate_skill_md() {
    let content = r#"---
name: test-skill
description: A test skill.
---

Instructions.
"#;
    let result = validate_skill_md(content);
    assert!(result.valid);
    assert_eq!(result.name.as_deref(), Some("test-skill"));
    assert!(result.errors.is_empty());
}

#[test]
fn test_validate_skill_md_invalid() {
    let content = r#"---
name: INVALID
---

Body.
"#;
    let result = validate_skill_md(content);
    assert!(!result.valid);
    assert!(!result.errors.is_empty());
}

#[test]
fn test_parse_argument_hint() {
    let content = r#"---
name: fix-issue
description: Fix a GitHub issue.
argument-hint: "<issue-number>"
---

Fix issue $ARGUMENTS.
"#;
    let parsed = parse_skill_md(content).unwrap();
    assert_eq!(parsed.argument_hint.as_deref(), Some("<issue-number>"));
}

// ========================================================================
// context and agent frontmatter tests
// ========================================================================

#[test]
fn test_parse_context_fork() {
    let content = r#"---
name: deep-research
description: Research a topic thoroughly.
context: fork
---

Research $ARGUMENTS.
"#;
    let parsed = parse_skill_md(content).unwrap();
    assert_eq!(parsed.context, SkillContext::Fork);
    assert!(parsed.agent.is_none());
}

#[test]
fn test_parse_context_fork_with_agent() {
    let content = r#"---
name: explore-code
description: Explore codebase.
context: fork
agent: Explore
---

Explore $ARGUMENTS.
"#;
    let parsed = parse_skill_md(content).unwrap();
    assert_eq!(parsed.context, SkillContext::Fork);
    assert_eq!(parsed.agent.as_deref(), Some("Explore"));
}

#[test]
fn test_parse_context_inline_explicit() {
    let content = r#"---
name: my-skill
description: A skill.
context: inline
---

Body.
"#;
    let parsed = parse_skill_md(content).unwrap();
    assert_eq!(parsed.context, SkillContext::Inline);
}

#[test]
fn test_parse_context_invalid_value() {
    let content = r#"---
name: my-skill
description: A skill.
context: parallel
---

Body.
"#;
    let err = parse_skill_md(content).unwrap_err();
    assert!(err.iter().any(|e| e.contains("context: invalid value")));
}

#[test]
fn test_parse_agent_without_fork_is_error() {
    let content = r#"---
name: my-skill
description: A skill.
agent: Explore
---

Body.
"#;
    let err = parse_skill_md(content).unwrap_err();
    assert!(
        err.iter()
            .any(|e| e.contains("agent: field is only meaningful"))
    );
}

#[test]
fn test_validate_warns_fork_without_agent() {
    let content = r#"---
name: my-skill
description: A skill.
context: fork
---

Body.
"#;
    let result = validate_skill_md(content);
    assert!(result.valid);
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.contains("general-purpose"))
    );
}

// -- model frontmatter tests --

#[test]
fn test_parse_model_with_fork() {
    let content = r#"---
name: quick-lint
description: Fast lint check.
context: fork
model: claude-haiku-4-5-20251001
---

Lint instructions.
"#;
    let parsed = parse_skill_md(content).unwrap();
    assert_eq!(parsed.model.as_deref(), Some("claude-haiku-4-5-20251001"));
    assert_eq!(parsed.context, SkillContext::Fork);
}

#[test]
fn test_parse_model_without_fork() {
    let content = r#"---
name: my-skill
description: A skill.
model: gpt-5.2
---

Body.
"#;
    let parsed = parse_skill_md(content).unwrap();
    assert_eq!(parsed.model.as_deref(), Some("gpt-5.2"));
    assert_eq!(parsed.context, SkillContext::Inline);
}

#[test]
fn test_validate_warns_model_without_fork() {
    let content = r#"---
name: my-skill
description: A skill.
model: gpt-5.2
---

Body.
"#;
    let result = validate_skill_md(content);
    assert!(result.valid);
    assert!(
        result
            .warnings
            .iter()
            .any(|w| w.contains("model:") && w.contains("context: fork"))
    );
}

#[test]
fn test_validate_no_warning_model_with_fork() {
    let content = r#"---
name: my-skill
description: A skill.
context: fork
agent: Explore
model: claude-haiku-4-5-20251001
---

Body.
"#;
    let result = validate_skill_md(content);
    assert!(result.valid);
    assert!(
        !result
            .warnings
            .iter()
            .any(|w| w.contains("model:") && w.contains("context: fork"))
    );
}

// ========================================================================
// expand_skill_arguments tests
// ========================================================================

#[test]
fn test_expand_full_arguments() {
    let content = "Process $ARGUMENTS now.";
    let result = expand_skill_arguments(content, "SearchBar React");
    assert_eq!(result, "Process SearchBar React now.");
}

#[test]
fn test_expand_indexed_arguments() {
    let content = "Migrate $ARGUMENTS[0] from $ARGUMENTS[1] to $ARGUMENTS[2].";
    let result = expand_skill_arguments(content, "SearchBar React Vue");
    assert_eq!(result, "Migrate SearchBar from React to Vue.");
}

#[test]
fn test_expand_shorthand_arguments() {
    let content = "Component: $0, from: $1, to: $2.";
    let result = expand_skill_arguments(content, "SearchBar React Vue");
    assert_eq!(result, "Component: SearchBar, from: React, to: Vue.");
}

#[test]
fn test_expand_quoted_arguments() {
    let content = "File: $0, message: $1.";
    let result = expand_skill_arguments(content, "app.js \"hello world\"");
    assert_eq!(result, "File: app.js, message: hello world.");
}

#[test]
fn test_expand_out_of_bounds() {
    let content = "A: $0, B: $1, C: $5.";
    let result = expand_skill_arguments(content, "only-one");
    assert_eq!(result, "A: only-one, B: , C: .");
}

#[test]
fn test_expand_no_placeholders_appends() {
    let content = "Do the thing.";
    let result = expand_skill_arguments(content, "some args");
    assert_eq!(result, "Do the thing.\n\nARGUMENTS: some args");
}

#[test]
fn test_expand_empty_args() {
    let content = "Content with $ARGUMENTS placeholder.";
    let result = expand_skill_arguments(content, "");
    assert_eq!(result, "Content with $ARGUMENTS placeholder.");
}

#[test]
fn test_expand_shorthand_no_word_collision() {
    // $NAME should NOT be replaced (not $0-$9 pattern)
    let content = "Variable $NAME and $0.";
    let result = expand_skill_arguments(content, "first");
    assert_eq!(result, "Variable $NAME and first.");
}

#[test]
fn test_expand_dollar_followed_by_multi_digit() {
    // $10 should NOT match $1 + "0" — only single-digit shorthand
    let content = "Value: $10 and $1.";
    let result = expand_skill_arguments(content, "a b");
    // $10 is not a valid shorthand (digit followed by digit), $1 = "b"
    assert_eq!(result, "Value: $10 and b.");
}

#[test]
fn test_split_skill_args_basic() {
    let args = split_skill_args("a b c");
    assert_eq!(args, vec!["a", "b", "c"]);
}

#[test]
fn test_split_skill_args_quoted() {
    let args = split_skill_args("\"hello world\" foo 'bar baz'");
    assert_eq!(args, vec!["hello world", "foo", "bar baz"]);
}

#[test]
fn test_split_skill_args_empty() {
    let args = split_skill_args("");
    assert!(args.is_empty());
}

#[test]
fn test_split_skill_args_extra_whitespace() {
    let args = split_skill_args("  a   b  ");
    assert_eq!(args, vec!["a", "b"]);
}

// ========================================================================
// substitute_activation_vars tests
// ========================================================================

// ========================================================================
// preprocess_command_injections tests
// ========================================================================

/// Mock executor for testing command injection preprocessing.
struct MockExecutor {
    responses: std::collections::HashMap<String, CommandResult>,
}

impl MockExecutor {
    fn new() -> Self {
        Self {
            responses: std::collections::HashMap::new(),
        }
    }

    fn add_response(&mut self, cmd: &str, stdout: &str, exit_code: i32) {
        self.responses.insert(
            cmd.to_string(),
            CommandResult {
                stdout: stdout.to_string(),
                exit_code,
            },
        );
    }
}

#[async_trait::async_trait]
impl CommandExecutor for MockExecutor {
    async fn execute_command(&self, command: &str) -> CommandResult {
        self.responses
            .get(command)
            .map(|r| CommandResult {
                stdout: r.stdout.clone(),
                exit_code: r.exit_code,
            })
            .unwrap_or(CommandResult {
                stdout: String::new(),
                exit_code: 127,
            })
    }
}

#[tokio::test]
async fn test_preprocess_single_command() {
    let mut exec = MockExecutor::new();
    exec.add_response("echo hello", "hello\n", 0);

    let content = "Output: !`echo hello`";
    let result = preprocess_command_injections(content, &exec).await;
    assert_eq!(result, "Output: hello");
}

#[tokio::test]
async fn test_preprocess_multiple_commands() {
    let mut exec = MockExecutor::new();
    exec.add_response("git status", "clean\n", 0);
    exec.add_response("date", "2026-03-19\n", 0);

    let content = "Status: !`git status`\nDate: !`date`";
    let result = preprocess_command_injections(content, &exec).await;
    assert_eq!(result, "Status: clean\nDate: 2026-03-19");
}

#[tokio::test]
async fn test_preprocess_command_failure() {
    let mut exec = MockExecutor::new();
    exec.add_response("bad-cmd", "error output\n", 1);

    let content = "Result: !`bad-cmd`";
    let result = preprocess_command_injections(content, &exec).await;
    assert_eq!(result, "Result: [Command failed: bad-cmd (exit code 1)]");
}

#[tokio::test]
async fn test_preprocess_empty_output() {
    let mut exec = MockExecutor::new();
    exec.add_response("true", "", 0);

    let content = "Result: !`true`";
    let result = preprocess_command_injections(content, &exec).await;
    assert_eq!(result, "Result: [No output]");
}

#[tokio::test]
async fn test_preprocess_no_commands() {
    let exec = MockExecutor::new();

    let content = "No commands here. Just `code` and text.";
    let result = preprocess_command_injections(content, &exec).await;
    assert_eq!(result, content);
}

#[tokio::test]
async fn test_preprocess_preserves_regular_backticks() {
    let mut exec = MockExecutor::new();
    exec.add_response("echo hi", "hi\n", 0);

    let content = "Use `code` and !`echo hi` here.";
    let result = preprocess_command_injections(content, &exec).await;
    assert_eq!(result, "Use `code` and hi here.");
}

#[tokio::test]
async fn test_preprocess_command_not_found() {
    let exec = MockExecutor::new(); // No responses registered

    let content = "Result: !`unknown-cmd`";
    let result = preprocess_command_injections(content, &exec).await;
    assert_eq!(
        result,
        "Result: [Command failed: unknown-cmd (exit code 127)]"
    );
}

// -- lenient YAML fallback tests --

#[test]
fn test_lenient_parse_unquoted_colon_in_description() {
    let content = r#"---
name: my-skill
description: Use this skill: it handles edge cases
---

Instructions.
"#;
    let parsed = parse_skill_md(content).unwrap();
    assert_eq!(parsed.name, "my-skill");
    assert_eq!(parsed.description, "Use this skill: it handles edge cases");
}

#[test]
fn test_parse_hash_inside_plain_value() {
    let content = "---\nname: my-skill\ndescription: Process C# files\n---\n\nBody.\n";
    let parsed = parse_skill_md(content).unwrap();
    assert_eq!(parsed.description, "Process C# files");
}

#[test]
fn test_parse_embedded_brackets_in_plain_value() {
    let content =
        "---\nname: my-skill\ndescription: Parse [markdown] and {templates}\n---\n\nBody.\n";
    let parsed = parse_skill_md(content).unwrap();
    assert_eq!(parsed.description, "Parse [markdown] and {templates}");
}

#[test]
fn test_parse_already_quoted_value_unchanged() {
    let content = "---\nname: my-skill\ndescription: \"Already quoted: value\"\n---\n\nBody.\n";
    let parsed = parse_skill_md(content).unwrap();
    assert_eq!(parsed.description, "Already quoted: value");
}

#[test]
fn test_fix_yaml_values_preserves_clean_yaml() {
    let input = "name: my-skill\ndescription: A simple skill";
    assert_eq!(fix_yaml_values(input), input);
}

#[test]
fn test_fix_yaml_values_quotes_colons() {
    let input = "name: my-skill\ndescription: Use this: it works";
    let fixed = fix_yaml_values(input);
    assert!(fixed.contains("description: \"Use this: it works\""));
}

#[test]
fn test_fix_yaml_values_escapes_inner_quotes() {
    let input = "name: my-skill\ndescription: Say \"hello\": world";
    let fixed = fix_yaml_values(input);
    assert!(fixed.contains(r#"description: "Say \"hello\": world""#));
}

#[test]
fn test_fix_yaml_values_skips_nested_keys() {
    let input = "metadata:\n  version: 1.0\n  key: value: nested";
    let fixed = fix_yaml_values(input);
    // Nested keys (indented) should not be modified
    assert_eq!(fixed, input);
}

#[test]
fn test_fix_yaml_values_preserves_flow_collections() {
    let input = "name: my-skill\nmetadata: { version: \"1.0\" }\ntags: [a, b]";
    let fixed = fix_yaml_values(input);
    assert_eq!(fixed, input);
}

#[test]
fn argument_values_are_inserted_literally_without_reexpansion() {
    assert_eq!(
        expand_skill_arguments("indexed: $ARGUMENTS[0]", "'$1' second"),
        "indexed: $1"
    );
    assert_eq!(
        expand_skill_arguments("full: $ARGUMENTS", "$1 second"),
        "full: $1 second"
    );
    assert_eq!(
        expand_skill_arguments("$ARGUMENTS[0] / $1", "'$ARGUMENTS' second"),
        "$ARGUMENTS / second"
    );
}

#[test]
fn quoted_empty_arguments_preserve_positional_identity() {
    assert_eq!(
        split_skill_args("\"\" next '' last"),
        vec!["", "next", "", "last"]
    );
    assert_eq!(
        expand_skill_arguments("$0|$1|$2|$3", "\"\" next '' last"),
        "|next||last"
    );
}

#[test]
fn invocation_flags_preserve_all_combinations_and_warn_only_when_unreachable() {
    for (user, disabled) in [(false, false), (false, true), (true, false), (true, true)] {
        let content = format!(
            "---\nname: sample\ndescription: sample\nuser-invocable: {user}\ndisable-model-invocation: {disabled}\n---\nBody."
        );
        let parsed = parse_skill_md(&content).unwrap();
        assert_eq!(
            (parsed.user_invocable, parsed.disable_model_invocation),
            (user, disabled)
        );
        let result = validate_skill_md(&content);
        assert!(result.valid);
        assert!(result.errors.is_empty());
        let expected = if !user && disabled {
            vec![
                "Skill is unreachable: user-invocable is false and disable-model-invocation is true. Neither users nor the model can invoke this skill.",
            ]
        } else {
            vec![]
        };
        assert_eq!(result.warnings, expected);
    }
}

#[test]
fn skill_context_wire_values_remain_portable() {
    for (value, wire) in [
        (SkillContext::Inline, "inline"),
        (SkillContext::Fork, "fork"),
    ] {
        assert_eq!(value.to_string(), wire);
        assert_eq!(
            serde_json::to_value(&value).unwrap(),
            serde_json::json!(wire)
        );
        assert_eq!(
            serde_json::from_value::<SkillContext>(serde_json::json!(wire)).unwrap(),
            value
        );
    }
    assert!(serde_json::from_str::<SkillContext>("\"other\"").is_err());
}

#[test]
fn activation_vars_replace_known_occurrences_and_preserve_other_text() {
    for dir in ["/home/user/skills/my-skill", "/.agents/skills/my-skill"] {
        assert_eq!(
            substitute_activation_vars(
                "${SKILL_DIR}/run ${SESSION_ID} ${SESSION_ID} ${OTHER}",
                "session_abc",
                dir
            ),
            format!("{dir}/run session_abc session_abc ${{OTHER}}")
        );
        assert_eq!(
            substitute_activation_vars("No variables. $SESSION_ID", "session_x", dir),
            "No variables. $SESSION_ID"
        );
    }
}

#[test]
fn parser_accepts_literal_byte_limits_and_rejects_the_next_byte() {
    for (field, limit) in [
        ("description", 1024),
        ("license", 500),
        ("compatibility", 500),
        ("argument-hint", 128),
    ] {
        for (value, valid) in [
            ("é".repeat(limit / 2), true),
            (format!("{}x", "é".repeat(limit / 2)), false),
        ] {
            let description = if field == "description" {
                String::new()
            } else {
                "description: sample\n".into()
            };
            let content = format!("---\nname: sample\n{description}{field}: {value}\n---\nBody.");
            let result = parse_skill_md(&content);
            assert_eq!(result.is_ok(), valid, "{field}");
            if !valid {
                assert_eq!(
                    result.unwrap_err(),
                    vec![format!("{field}: exceeds {limit} character limit")]
                );
            }
        }
    }
    assert!(validate_skill_name(&"a".repeat(64)).is_ok());
    assert_eq!(
        validate_skill_name(&"a".repeat(65)).unwrap_err(),
        vec!["name: must be 1-64 characters"]
    );
    for (size, valid) in [(102400, true), (102401, false)] {
        let result = parse_skill_md(&format!(
            "---\nname: sample\ndescription: sample\n---\n{}",
            "x".repeat(size)
        ));
        assert_eq!(result.is_ok(), valid);
        if !valid {
            assert_eq!(
                result.unwrap_err(),
                vec!["instructions: exceeds 100 KB limit"]
            );
        }
    }
}

#[test]
fn validation_line_warning_starts_after_five_hundred_lines() {
    for lines in [500, 501] {
        let result = validate_skill_md(&format!(
            "---\nname: sample\ndescription: sample\n---\n{}",
            "line\n".repeat(lines)
        ));
        assert!(result.valid);
        assert!(result.errors.is_empty());
        let expected = if lines == 501 {
            vec!["Instructions exceed 500 lines (501 lines). Consider splitting into references."]
        } else {
            vec![]
        };
        assert_eq!(result.warnings, expected);
    }
}

#[tokio::test(start_paused = true)]
async fn command_preprocessing_bounds_fanout_and_preserves_result_order() {
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    struct Executor {
        active: AtomicUsize,
        peak: AtomicUsize,
        calls: Mutex<Vec<usize>>,
    }
    #[async_trait::async_trait]
    impl CommandExecutor for Executor {
        async fn execute_command(&self, command: &str) -> CommandResult {
            let index: usize = command.parse().unwrap();
            self.calls.lock().unwrap().push(index);
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(active, Ordering::SeqCst);
            tokio::time::sleep(std::time::Duration::from_millis((35 - index) as u64)).await;
            self.active.fetch_sub(1, Ordering::SeqCst);
            CommandResult {
                stdout: format!("value-{index}\n"),
                exit_code: 0,
            }
        }
    }
    let executor = Executor {
        active: AtomicUsize::new(0),
        peak: AtomicUsize::new(0),
        calls: Mutex::new(vec![]),
    };
    let content = (0..34)
        .map(|i| format!("界!`{i}`"))
        .collect::<Vec<_>>()
        .join("|");
    let expected = (0..34)
        .map(|i| {
            if i < 32 {
                format!("界value-{i}")
            } else {
                "界[Too many command placeholders: limit is 32]".into()
            }
        })
        .collect::<Vec<_>>()
        .join("|");
    assert_eq!(
        preprocess_command_injections(&content, &executor).await,
        expected
    );
    assert_eq!(executor.peak.load(Ordering::SeqCst), 4);
    assert_eq!(executor.active.load(Ordering::SeqCst), 0);
    let mut calls = executor.calls.into_inner().unwrap();
    calls.sort();
    assert_eq!(calls, (0..32).collect::<Vec<_>>());
}
