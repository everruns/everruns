use super::*;

#[test]
fn test_apply_harness_detects_strict_id() {
    let id = "harness_00000000000000000000000000000001";
    let req = apply_harness(CreateAgentRequest::new("a", "p"), Some(id.to_string()));
    assert_eq!(req.harness_id.as_deref(), Some(id));
    assert!(req.harness_name.is_none());
}

#[test]
fn test_apply_harness_treats_bare_name_as_name() {
    let req = apply_harness(
        CreateAgentRequest::new("a", "p"),
        Some("generic".to_string()),
    );
    assert_eq!(req.harness_name.as_deref(), Some("generic"));
    assert!(req.harness_id.is_none());
}

#[test]
fn test_apply_harness_keeps_prefix_names_as_name() {
    // "harness_generic" is not a strict harness id (not 32 hex), so it is a name.
    let req = apply_harness(
        CreateAgentRequest::new("a", "p"),
        Some("harness_generic".to_string()),
    );
    assert_eq!(req.harness_name.as_deref(), Some("harness_generic"));
    assert!(req.harness_id.is_none());
}

#[test]
fn test_apply_harness_none_sets_nothing() {
    let req = apply_harness(CreateAgentRequest::new("a", "p"), None);
    assert!(req.harness_id.is_none());
    assert!(req.harness_name.is_none());
}

#[test]
fn test_parse_agent_file_json() {
    let content = r#"{"name":"test","system_prompt":"hello"}"#;
    let val = parse_agent_file_as_json(Path::new("agent.json"), content).unwrap();
    assert_eq!(val["name"], "test");
    assert_eq!(val["system_prompt"], "hello");
}

#[test]
fn test_parse_agent_file_yaml() {
    let content = "name: test\nsystem_prompt: hello\n";
    let val = parse_agent_file_as_json(Path::new("agent.yaml"), content).unwrap();
    assert_eq!(val["name"], "test");
    assert_eq!(val["system_prompt"], "hello");
}

#[test]
fn test_parse_agent_file_toml() {
    let content = "name = \"test\"\nsystem_prompt = \"hello\"\n";
    let val = parse_agent_file_as_json(Path::new("agent.toml"), content).unwrap();
    assert_eq!(val["name"], "test");
    assert_eq!(val["system_prompt"], "hello");
}

#[test]
fn test_parse_agent_file_markdown() {
    let content = "---\nname: test\n---\nHello world";
    let val = parse_agent_file_as_json(Path::new("agent.md"), content).unwrap();
    assert_eq!(val["name"], "test");
    assert_eq!(val["system_prompt"], "Hello world");
}

#[test]
fn test_parse_agent_file_markdown_with_system_prompt() {
    let content = "---\nname: test\nsystem_prompt: from frontmatter\n---\nBody text";
    let val = parse_agent_file_as_json(Path::new("agent.md"), content).unwrap();
    assert_eq!(val["name"], "test");
    // Frontmatter system_prompt takes precedence
    assert_eq!(val["system_prompt"], "from frontmatter");
}

#[test]
fn test_glob_initial_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "world").unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub/nested.txt"), "content").unwrap();

    // Hidden files/dirs should be skipped (security: prevents .env, .ssh leaks)
    std::fs::write(dir.path().join(".env"), "SECRET=key").unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    std::fs::write(dir.path().join(".git/config"), "gitdata").unwrap();

    let files = glob_initial_files(dir.path().to_str().unwrap(), false, &[]).unwrap();
    assert_eq!(files.len(), 2);
    assert!(files.iter().any(|f| f.path == "/workspace/hello.txt"));
    assert!(files.iter().any(|f| f.path == "/workspace/sub/nested.txt"));
    assert!(files.iter().all(|f| f.is_readonly));
    assert!(files.iter().all(|f| f.encoding == "text"));
}

#[test]
fn test_glob_initial_files_writable() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "world").unwrap();

    let files = glob_initial_files(dir.path().to_str().unwrap(), true, &[]).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "/workspace/hello.txt");
    assert!(!files[0].is_readonly);
}

#[test]
fn test_glob_initial_files_includes_dot_agents() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("agent.md"), "# Agent").unwrap();
    std::fs::create_dir_all(dir.path().join(".agents")).unwrap();
    std::fs::write(dir.path().join(".agents/config.json"), "{}").unwrap();

    let files = glob_initial_files(dir.path().to_str().unwrap(), false, &[]).unwrap();
    assert_eq!(files.len(), 2);
    assert!(files.iter().any(|f| f.path == "/workspace/agent.md"));
    assert!(
        files
            .iter()
            .any(|f| f.path == "/workspace/.agents/config.json")
    );
}

#[test]
fn test_glob_initial_files_empty_dir() {
    let dir = tempfile::tempdir().unwrap();
    let result = glob_initial_files(dir.path().to_str().unwrap(), false, &[]);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("No files found"));
}

#[test]
fn test_glob_initial_files_not_a_dir() {
    let dir = tempfile::tempdir().unwrap();
    let file_path = dir.path().join("file.txt");
    std::fs::write(&file_path, "content").unwrap();
    let result = glob_initial_files(file_path.to_str().unwrap(), false, &[]);
    assert!(result.is_err());
    assert!(result.unwrap_err().to_string().contains("Not a directory"));
}

#[test]
fn test_glob_initial_files_skips_symlinks_outside_base() {
    let dir = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();
    // Create a symlink pointing outside the base directory
    std::os::unix::fs::symlink(
        outside.path().join("secret.txt"),
        dir.path().join("link.txt"),
    )
    .unwrap();
    // Should error because only file is the symlink (skipped)
    let result = glob_initial_files(dir.path().to_str().unwrap(), false, &[]);
    assert!(result.is_err());
}

#[test]
fn test_parse_agent_file_non_object_errors() {
    let content = "just a string";
    let result = parse_agent_file_as_json(Path::new("agent.yaml"), content);
    assert!(result.is_err());
}

#[test]
fn test_has_initial_files_globs_with_strings() {
    let content = "---\nname: test\ninitial_files:\n  - .\n  - .agents/*\n---\nPrompt";
    let agent = parse_agent_file_as_json(Path::new("agent.md"), content).unwrap();
    assert!(initial_files_has_globs(&agent));
}

#[test]
fn test_has_initial_files_globs_without_strings() {
    // No initial_files at all
    let content = "---\nname: test\n---\nPrompt";
    let agent = parse_agent_file_as_json(Path::new("agent.md"), content).unwrap();
    assert!(!initial_files_has_globs(&agent));

    // initial_files with objects (already expanded)
    let content = r#"{"name":"test","initial_files":[{"path":"/workspace/f.txt","content":"x","encoding":"text","is_readonly":false}]}"#;
    let agent = parse_agent_file_as_json(Path::new("agent.json"), content).unwrap();
    assert!(!initial_files_has_globs(&agent));
}

#[test]
fn test_has_initial_files_globs_toml() {
    let content = "name = \"test\"\ninitial_files = [\".\", \".agents/*\"]\n";
    let agent = parse_agent_file_as_json(Path::new("agent.toml"), content).unwrap();
    assert!(initial_files_has_globs(&agent));
}

#[test]
fn test_expand_initial_files_globs_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "world").unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub/nested.txt"), "content").unwrap();

    let agent = serde_json::json!({
        "name": "test",
        "initial_files": ["."]
    });

    let files = expand_initial_files_globs(&agent, dir.path(), false).unwrap();
    assert_eq!(files.len(), 2);
    assert!(files.iter().any(|f| f.path == "/workspace/hello.txt"));
    assert!(files.iter().any(|f| f.path == "/workspace/sub/nested.txt"));
    assert!(files.iter().all(|f| f.is_readonly));
}

#[test]
fn test_expand_initial_files_globs_subdirectory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.txt"), "root").unwrap();
    std::fs::create_dir_all(dir.path().join(".agents")).unwrap();
    std::fs::write(dir.path().join(".agents/config.json"), "{}").unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub/nested.txt"), "nested").unwrap();

    // .agents subdirectory preserves prefix in workspace path
    let agent = serde_json::json!({
        "name": "test",
        "initial_files": [".agents"]
    });

    let files = expand_initial_files_globs(&agent, dir.path(), false).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "/workspace/.agents/config.json");

    // Regular subdirectory also preserves prefix
    let agent2 = serde_json::json!({
        "name": "test",
        "initial_files": ["sub"]
    });

    let files2 = expand_initial_files_globs(&agent2, dir.path(), false).unwrap();
    assert_eq!(files2.len(), 1);
    assert_eq!(files2[0].path, "/workspace/sub/nested.txt");
}

#[test]
fn test_expand_initial_files_globs_with_wildcard() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("root.txt"), "root").unwrap();
    std::fs::create_dir_all(dir.path().join(".agents")).unwrap();
    std::fs::write(dir.path().join(".agents/config.json"), "{}").unwrap();

    // ".agents/*" should work — the * is stripped, .agents/ is walked
    let agent = serde_json::json!({
        "name": "test",
        "initial_files": [".agents/*"]
    });

    let files = expand_initial_files_globs(&agent, dir.path(), false).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "/workspace/.agents/config.json");
}

#[test]
fn test_expand_initial_files_globs_deduplicates() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "world").unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub/readme.md"), "# Hi").unwrap();

    // Both "." entries collect the same files — dedup by workspace path
    let agent = serde_json::json!({
        "name": "test",
        "initial_files": [".", "."]
    });

    let files = expand_initial_files_globs(&agent, dir.path(), false).unwrap();
    let hello_count = files
        .iter()
        .filter(|f| f.path.contains("hello.txt"))
        .count();
    assert_eq!(hello_count, 1);
    assert_eq!(files.len(), 2);

    // "." and "sub" overlap on sub/readme.md — dedup keeps first occurrence
    let agent2 = serde_json::json!({
        "name": "test",
        "initial_files": [".", "sub"]
    });

    let files2 = expand_initial_files_globs(&agent2, dir.path(), false).unwrap();
    assert_eq!(files2.len(), 2);
    assert!(files2.iter().any(|f| f.path == "/workspace/hello.txt"));
    assert!(files2.iter().any(|f| f.path == "/workspace/sub/readme.md"));
}

#[test]
fn test_expand_initial_files_globs_single_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("README.md"), "# Readme").unwrap();
    std::fs::write(dir.path().join("other.txt"), "other").unwrap();

    let agent = serde_json::json!({
        "name": "test",
        "initial_files": ["README.md"]
    });

    let files = expand_initial_files_globs(&agent, dir.path(), false).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "/workspace/README.md");
    assert_eq!(files[0].content, "# Readme");
}

#[test]
fn test_expand_initial_files_globs_writable() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "world").unwrap();

    let agent = serde_json::json!({
        "name": "test",
        "initial_files": ["."]
    });

    let files = expand_initial_files_globs(&agent, dir.path(), true).unwrap();
    assert!(files.iter().all(|f| !f.is_readonly));
}

#[test]
fn test_expand_initial_files_globs_nonexistent_errors() {
    let dir = tempfile::tempdir().unwrap();

    let agent = serde_json::json!({
        "name": "test",
        "initial_files": ["nonexistent"]
    });

    let result = expand_initial_files_globs(&agent, dir.path(), false);
    assert!(result.is_err());
}

#[test]
fn test_expand_initial_files_rejects_hidden_single_file() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".env"), "SECRET=key").unwrap();
    std::fs::write(dir.path().join("ok.txt"), "safe").unwrap();

    // Explicitly listing .env should be rejected (hidden, not in allowlist)
    let agent = serde_json::json!({
        "name": "test",
        "initial_files": [".env", "ok.txt"]
    });

    let files = expand_initial_files_globs(&agent, dir.path(), false).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "/workspace/ok.txt");
}

#[test]
fn test_expand_initial_files_rejects_hidden_directory() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".ssh")).unwrap();
    std::fs::write(dir.path().join(".ssh/config"), "Host *").unwrap();
    std::fs::write(dir.path().join("ok.txt"), "safe").unwrap();

    let agent = serde_json::json!({
        "name": "test",
        "initial_files": [".ssh", "ok.txt"]
    });

    let files = expand_initial_files_globs(&agent, dir.path(), false).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "/workspace/ok.txt");
}

#[test]
fn test_glob_base_dir() {
    assert_eq!(glob_base_dir("."), ".");
    assert_eq!(glob_base_dir(".agents"), ".agents");
    assert_eq!(glob_base_dir(".agents/*"), ".agents");
    assert_eq!(glob_base_dir("src/**/*.rs"), "src");
    assert_eq!(glob_base_dir("*.txt"), "");
    assert_eq!(glob_base_dir("dir/sub/*.md"), "dir/sub");
}

#[test]
fn test_expand_initial_files_includes_default_dev_dot_dirs() {
    // Common dev-ecosystem dot directories ship by default without
    // requiring `initial_files_allow_hidden`. This is only about file
    // collection policy; the CLI does not interpret tool-specific files.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".agents")).unwrap();
    std::fs::write(dir.path().join(".agents/config.json"), "{}").unwrap();
    std::fs::create_dir_all(dir.path().join(".github/workflows")).unwrap();
    std::fs::write(dir.path().join(".github/workflows/ci.yml"), "name: ci").unwrap();
    std::fs::create_dir_all(dir.path().join(".vscode")).unwrap();
    std::fs::write(
        dir.path().join(".vscode/settings.json"),
        "{\"editor.tabSize\":2}",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
    std::fs::write(
        dir.path().join(".claude/settings.json"),
        "{\"permissions\":{}}",
    )
    .unwrap();

    for dot_path in [".agents", ".github", ".vscode", ".claude"] {
        let agent = serde_json::json!({
            "name": "test",
            "initial_files": [dot_path]
        });
        let files = expand_initial_files_globs(&agent, dir.path(), false).unwrap();
        assert!(
            !files.is_empty(),
            "expected files when shipping {dot_path}, got none"
        );
        assert!(
            files.iter().all(|f| f.path.starts_with("/workspace/")),
            "all collected files must be under /workspace"
        );
    }
}

#[test]
fn test_expand_initial_files_user_opt_in_extras() {
    // User-declared `initial_files_allow_hidden` extends the allowlist.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".mytool")).unwrap();
    std::fs::write(dir.path().join(".mytool/config"), "k=v").unwrap();

    // Without opt-in: hidden directory is skipped.
    let baseline = serde_json::json!({
        "name": "test",
        "initial_files": [".mytool", "."]
    });
    std::fs::write(dir.path().join("ok.txt"), "ok").unwrap();
    let baseline_files = expand_initial_files_globs(&baseline, dir.path(), false).unwrap();
    assert!(
        baseline_files.iter().all(|f| !f.path.contains("/.mytool/")),
        "baseline must not include .mytool"
    );

    // With opt-in: .mytool is collected.
    let opt_in = serde_json::json!({
        "name": "test",
        "initial_files": [".mytool"],
        "initial_files_allow_hidden": [".mytool"]
    });
    let opt_in_files = expand_initial_files_globs(&opt_in, dir.path(), false).unwrap();
    assert!(
        opt_in_files
            .iter()
            .any(|f| f.path == "/workspace/.mytool/config"),
        "opt-in must include .mytool/config, got: {:?}",
        opt_in_files.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
}

#[test]
fn test_expand_initial_files_hard_deny_blocks_opt_in() {
    // Even if a user opts in, .ssh/.env are always rejected.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".ssh")).unwrap();
    std::fs::write(dir.path().join(".ssh/config"), "Host *").unwrap();
    std::fs::write(dir.path().join(".env"), "SECRET=key").unwrap();
    std::fs::write(dir.path().join("ok.txt"), "safe").unwrap();

    let agent = serde_json::json!({
        "name": "test",
        "initial_files": [".ssh", ".env", "ok.txt"],
        "initial_files_allow_hidden": [".ssh", ".env"]
    });

    let files = expand_initial_files_globs(&agent, dir.path(), false).unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].path, "/workspace/ok.txt");
}

#[test]
fn test_extract_allow_hidden_extras_filters() {
    // Non-hidden, denied, and non-string entries are filtered out.
    let agent = serde_json::json!({
        "initial_files_allow_hidden": [
            ".mytool",   // kept
            "regular",   // dropped: not hidden
            ".ssh",      // dropped: hard-denied
            ".env",      // dropped: hard-denied
            42           // dropped: not a string
        ]
    });
    let extras = extract_allow_hidden_extras(&agent);
    assert_eq!(extras, vec![".mytool".to_string()]);
}

#[test]
fn test_is_disallowed_hidden_with_extras_and_deny() {
    // No extras: built-in allowlist + deny floor.
    let base = HiddenPathPolicy::new(&[]);
    assert!(!base.component_is_disallowed(".github"));
    assert!(!base.component_is_disallowed(".claude"));
    assert!(base.component_is_disallowed(".env"));
    assert!(base.component_is_disallowed(".ssh"));

    // User opt-in extends allowlist.
    let with_extras = HiddenPathPolicy::new(&[".mytool".to_string()]);
    assert!(!with_extras.component_is_disallowed(".mytool"));

    // Deny floor is unconditional, even if extras try to override it.
    let bypass = HiddenPathPolicy::new(&[".ssh".to_string(), ".env".to_string()]);
    assert!(bypass.component_is_disallowed(".ssh"));
    assert!(bypass.component_is_disallowed(".env"));

    // Non-hidden components are never gated.
    assert!(!base.component_is_disallowed("regular"));
    assert!(!base.component_is_disallowed(""));

    // Nested deny components are caught by path-level check.
    assert!(base.path_has_disallowed_component(Path::new(".github/.env")));
    assert!(base.path_has_disallowed_component(Path::new(".claude/.ssh/config")));
}

#[test]
fn test_is_valid_basename_extra_rejects_paths() {
    // Reject path separators, relative-path placeholders, and missing dot.
    assert!(!is_valid_basename_extra(""));
    assert!(!is_valid_basename_extra("."));
    assert!(!is_valid_basename_extra(".."));
    assert!(!is_valid_basename_extra("regular"));
    assert!(!is_valid_basename_extra(".mytool/sub"));
    assert!(!is_valid_basename_extra(".mytool\\sub"));
    assert!(!is_valid_basename_extra("../escape"));
    assert!(is_valid_basename_extra(".mytool"));
    assert!(is_valid_basename_extra(".otherproj"));
}

#[test]
fn test_extract_allow_hidden_extras_rejects_path_separators() {
    let agent = serde_json::json!({
        "initial_files_allow_hidden": [
            ".mytool",
            ".otherproj/sub",   // dropped: contains /
            ".escape\\windows", // dropped: contains \
            ".",                // dropped: relative-path placeholder
            "..",               // dropped: relative-path placeholder
        ]
    });
    let extras = extract_allow_hidden_extras(&agent);
    assert_eq!(extras, vec![".mytool".to_string()]);
}

#[test]
fn test_collect_dir_files_rejects_nested_denied_under_allowed_root() {
    // .github/.env must NOT be collected even though .github is allowed.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join(".github/workflows")).unwrap();
    std::fs::write(dir.path().join(".github/workflows/ci.yml"), "name: ci").unwrap();
    std::fs::write(dir.path().join(".github/.env"), "SECRET=key").unwrap();

    let agent = serde_json::json!({
        "name": "test",
        "initial_files": [".github"]
    });

    let files = expand_initial_files_globs(&agent, dir.path(), false).unwrap();
    assert!(
        files
            .iter()
            .any(|f| f.path == "/workspace/.github/workflows/ci.yml"),
        "ci.yml should be collected"
    );
    assert!(
        files.iter().all(|f| !f.path.contains("/.env")),
        ".env nested under .github must NOT be collected, got: {:?}",
        files.iter().map(|f| &f.path).collect::<Vec<_>>()
    );
}

#[test]
fn test_glob_initial_files_extras_walks_user_dot_dir() {
    // glob_initial_files (the `--initial-files-dir` path) honors extras too.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("hello.txt"), "world").unwrap();
    std::fs::create_dir_all(dir.path().join(".mytool")).unwrap();
    std::fs::write(dir.path().join(".mytool/config"), "k=v").unwrap();

    // Without opt-in: .mytool dropped.
    let baseline = glob_initial_files(dir.path().to_str().unwrap(), false, &[]).unwrap();
    assert!(
        baseline.iter().all(|f| !f.path.contains("/.mytool/")),
        "baseline should not include .mytool"
    );

    // With opt-in: .mytool walked.
    let extras = vec![".mytool".to_string()];
    let opt_in = glob_initial_files(dir.path().to_str().unwrap(), false, &extras).unwrap();
    assert!(
        opt_in.iter().any(|f| f.path == "/workspace/.mytool/config"),
        "opt-in must include .mytool/config"
    );
}
