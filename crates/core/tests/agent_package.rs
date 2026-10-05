#![allow(clippy::unwrap_used, clippy::expect_used)]
use everruns_core::agent_package::{AgentPackage, Format};

#[test]
fn legacy_markdown_keeps_prompt_and_capability_config() {
    let package = AgentPackage::parse(
        "---\nname: Dad Jokes\ncapabilities:\n  - current_time\n---\nTell a joke.\n",
        Format::Markdown,
    )
    .unwrap();
    assert_eq!(package.manifest.name, "dad-jokes");
    assert_eq!(package.manifest.instructions, "Tell a joke.\n");
    assert_eq!(
        package.manifest.capabilities[0].capability_id(),
        "current_time"
    );
    assert!(
        !package
            .to_string(Format::Markdown)
            .unwrap()
            .contains("instructions")
    );
}

#[test]
fn malformed_documents_and_unknown_fields_do_not_become_prompts() {
    for text in [
        "{ broken",
        "---\nname: [\n---\nHi",
        "schema_version = 1\nname = 'test'\ninstructions = 'Hi'\nmax_iteratons = 2",
    ] {
        assert!(AgentPackage::parse(text, Format::Auto).is_err(), "{text}");
    }
    assert!(
        AgentPackage::parse(
            "schema_version = 9\nname = 'test'\ninstructions = 'Hi'",
            Format::Toml
        )
        .is_err()
    );
    assert!(
        AgentPackage::parse(
            "name: test\ninstructions: One\nsystem_prompt: Two",
            Format::Yaml
        )
        .is_err()
    );
}

#[test]
fn id_free_export_and_all_formats_round_trip() {
    let package = AgentPackage::parse(r#"{"name":"test","instructions":"Line one\nLine two\n","max_iterations":7,"parallel_tool_calls":false,"capabilities":[{"ref":"current_time","config":{}}],"initial_files":[{"path":"/notes.txt","content":"hello","is_readonly":true}],"mcpServers":{"docs":{"type":"http","url":"https://example.org/mcp"}}}"#, Format::Json).unwrap();
    for format in [Format::Markdown, Format::Json, Format::Yaml, Format::Toml] {
        let text = package.to_string(format).unwrap();
        assert!(!text.contains("system_prompt"));
        assert!(!text.contains("\"id\""));
        let restored = AgentPackage::parse(&text, format).unwrap();
        assert!(
            package.diff(&restored).unwrap().is_empty(),
            "{format:?}: {text}"
        );
    }
}

#[test]
fn folder_zip_and_export_preserve_binary_assets_and_entire_skills() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.path().join("skills/investigate/scripts")).unwrap();
    std::fs::create_dir_all(root.path().join("files")).unwrap();
    std::fs::write(
        root.path().join("agent.toml"),
        "schema_version = 1\nname = 'test'\n",
    )
    .unwrap();
    std::fs::write(root.path().join("instructions.md"), "Investigate.\n").unwrap();
    std::fs::write(
        root.path().join("skills/investigate/SKILL.md"),
        "---\nname: investigate\ndescription: Investigate issues\n---\nRead scripts/check.py",
    )
    .unwrap();
    std::fs::write(
        root.path().join("skills/investigate/scripts/check.py"),
        "print('ok')",
    )
    .unwrap();
    std::fs::write(root.path().join("files/sample.bin"), [0, 255, 10]).unwrap();
    let package = AgentPackage::load(root.path()).unwrap();
    let from_manifest = AgentPackage::load(root.path().join("agent.toml")).unwrap();
    assert!(package.diff(&from_manifest).unwrap().is_empty());
    assert_eq!(package.files().unwrap().len(), 3);
    assert!(package.files().unwrap().iter().all(|f| f.is_readonly));
    assert!(
        package
            .files()
            .unwrap()
            .iter()
            .any(|f| f.path == "/.agents/skills/investigate/scripts/check.py")
    );
    let archive = package.to_zip().unwrap();
    assert!(
        package
            .diff(&AgentPackage::from_zip(&archive).unwrap())
            .unwrap()
            .is_empty()
    );
    let destination = tempfile::tempdir().unwrap();
    package.write_folder(destination.path()).unwrap();
    assert!(
        package
            .diff(&AgentPackage::load(destination.path()).unwrap())
            .unwrap()
            .is_empty()
    );
    assert!(
        package.write_folder(destination.path()).is_err(),
        "existing files must not be overwritten"
    );
}

#[test]
fn semantic_diff_ignores_order_and_shows_changed_files_without_contents() {
    let left = AgentPackage::parse(r#"{"name":"test","instructions":"Hi","tags":["a","b"],"initial_files":[{"path":"/a","content":"secret-looking-data"}]}"#, Format::Json).unwrap();
    let right = AgentPackage::parse(r#"{"name":"test","instructions":"Hi","tags":["b","a"],"initial_files":[{"path":"/a","content":"changed"}]}"#, Format::Json).unwrap();
    let diff = left.diff(&right).unwrap();
    assert_eq!(diff.len(), 1);
    assert!(diff[0].path.contains("files"));
    assert!(
        !serde_json::to_string(&diff)
            .unwrap()
            .contains("secret-looking-data")
    );
}

#[test]
fn invalid_paths_duplicate_assets_and_unresolved_sources_fail() {
    for files in [
        r#"[{"path":"../outside","content":"x"}]"#,
        r#"[{"path":"/a","content":"x"},{"path":"/workspace/a","content":"y"}]"#,
        r#"[{"path":"/a","content":"!","encoding":"base64"}]"#,
    ] {
        assert!(
            AgentPackage::parse(
                &format!(r#"{{"name":"test","instructions":"Hi","initial_files":{files}}}"#),
                Format::Json
            )
            .is_err()
        );
    }
    let package = AgentPackage::parse(
        "name: test\ninstructions: Hi\ninitial_files: ['files/**']",
        Format::Yaml,
    )
    .unwrap();
    assert!(package.files().is_err());
}

#[test]
fn channel_descriptions_validate_and_default_to_disabled() {
    let package = AgentPackage::parse(
        "schema_version = 1\nname = 'test'\ninstructions = 'Hi'\n[channels.chat]\ntype = 'ag_ui'\n",
        Format::Toml,
    )
    .unwrap();
    assert!(!package.manifest.channels["chat"].enabled);
    let enabled = AgentPackage::parse(
        "name = 'test'\ninstructions = 'Hi'\n[channels.chat]\ntype = 'ag_ui'\nenabled = true",
        Format::Toml,
    )
    .unwrap();
    assert!(enabled.manifest.channels["chat"].enabled);

    for extra in [
        "type = 'typo'",
        "type = 'ag_ui'\n[channels.chat.config]\ntoken = 'secret'",
    ] {
        assert!(
            AgentPackage::parse(
                &format!("name = 'test'\ninstructions = 'Hi'\n[channels.chat]\n{extra}"),
                Format::Toml
            )
            .is_err()
        );
    }
}

#[test]
fn credential_bindings_and_nested_unknown_fields_validate() {
    for extra in [
        r#""initial_files":[{"path":"/x","content":"x","readonly":false}]"#,
        r#""capabilities":[{"ref":"current_time","confgi":{}}]"#,
        r#""model":{"provider":"openai","model":"test","api_key":"secret"}"#,
        r#""network_access":{"blockd":["169.254.169.254"]}"#,
        r#""tools":[{"type":"client_side","name":"ping","description":"x","parameters":{"type":"object"},"polciy":"requires_approval"}]"#,
        r#""mcpServers":{"docs":{"url":"https://example.org/mcp","heders":{}}}"#,
        r#""mcpServers":{"docs":{"url":"https://user:password@example.org/mcp"}}"#,
        r#""mcpServers":{"docs":{"url":"https://example.org/mcp?api_key=secret"}}"#,
        r#""mcpServers":{"docs":{"url":"https://example.org/mcp","headers":{"Authorization":"secret"}}}"#,
    ] {
        assert!(
            AgentPackage::parse(
                &format!(r#"{{"name":"test","instructions":"Hi",{extra}}}"#),
                Format::Json
            )
            .is_err(),
            "{extra}"
        );
    }
    let package = AgentPackage::parse(r#"{"name":"test","instructions":"Hi","mcpServers":{"docs":{"url":"https://example.org/mcp","headers":{"Authorization":"${DOCS_TOKEN}"}}}}"#, Format::Json).unwrap();
    assert!(package.bind_mcp(|_| None).is_err());
    assert_eq!(
        package
            .bind_mcp(|name| (name == "DOCS_TOKEN").then(|| "Bearer token".into()))
            .unwrap()["docs"]
            .headers["Authorization"],
        "Bearer token"
    );
    assert!(
        package
            .to_string(Format::Json)
            .unwrap()
            .contains("${DOCS_TOKEN}")
    );
}

#[test]
fn archives_reject_traversal_symlinks_and_expansion_bombs() {
    use std::io::{Cursor, Write};
    for (path, content) in [
        ("../agent.toml", vec![b'x']),
        ("large.bin", vec![b'x'; 1024 * 1024 + 1]),
    ] {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(path, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(&content).unwrap();
        assert!(AgentPackage::from_zip(&writer.finish().unwrap().into_inner()).is_err());
    }
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer
        .add_symlink(
            "agent.toml",
            "../outside",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    assert!(AgentPackage::from_zip(&writer.finish().unwrap().into_inner()).is_err());
}

#[cfg(unix)]
#[test]
fn ancestor_symlinks_cannot_escape_source_or_instructions() {
    let package = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();
    std::os::unix::fs::symlink(outside.path(), package.path().join("link")).unwrap();
    for definition in [
        "name = 'test'\ninstructions = 'Hi'\ninitial_files = ['link/secret.txt']",
        "name = 'test'\ninstructions_file = 'link/secret.txt'",
    ] {
        std::fs::write(package.path().join("agent.toml"), definition).unwrap();
        assert!(AgentPackage::load(package.path()).is_err());
    }
}

#[test]
fn malformed_skills_and_tool_definitions_fail_validation() {
    for extra in [
        r#""tools":[{"type":"client_side","name":"bad name","description":"x","parameters":{}}]"#,
        r#""tools":[{"type":"builtin","name":"ping","description":"x","parameters":{"type":"object"}}]"#,
        r#""initial_files":[{"path":"/.agents/skills/check/SKILL.md","content":"No frontmatter"}]"#,
    ] {
        assert!(
            AgentPackage::parse(
                &format!(r#"{{"name":"test","instructions":"Hi",{extra}}}"#),
                Format::Json
            )
            .is_err(),
            "{extra}"
        );
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("agent.toml"),
        "name = 'test'\ninstructions = 'Hi'",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("skills/check/scripts")).unwrap();
    std::fs::write(
        dir.path().join("skills/check/scripts/check.py"),
        "print('check')",
    )
    .unwrap();
    assert!(
        AgentPackage::load(dir.path())
            .unwrap_err()
            .to_string()
            .contains("requires SKILL.md")
    );
}

#[test]
fn hidden_inline_files_round_trip_without_reading_hidden_host_files() {
    let package = AgentPackage::parse(
        r#"{"name":"test","instructions":"Hi","initial_files":[{"path":"/.env","content":"EXAMPLE=value","is_readonly":false}]}"#,
        Format::Json,
    ).unwrap();
    let entries = package.folder_entries().unwrap();
    assert!(!entries.contains_key("files/.env"));
    let restored = AgentPackage::from_zip(&package.to_zip().unwrap()).unwrap();
    assert!(package.diff(&restored).unwrap().is_empty());
    let dir = tempfile::tempdir().unwrap();
    package.write_folder(dir.path()).unwrap();
    assert!(
        package
            .diff(&AgentPackage::load(dir.path()).unwrap())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn folder_collection_uses_the_cli_hidden_file_policy() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("agent.toml"),
        "name = 'test'\ninstructions = 'Hi'",
    )
    .unwrap();
    std::fs::create_dir(dir.path().join("files")).unwrap();
    for (path, content) in [
        (".gitignore", "target/"),
        (".editorconfig", "root = true"),
        (".env", "TOKEN=secret"),
    ] {
        std::fs::write(dir.path().join("files").join(path), content).unwrap();
    }
    let package = AgentPackage::load(dir.path()).unwrap();
    let files = package.files().unwrap();
    assert_eq!(files.len(), 2);
    assert!(files.iter().any(|file| file.path == "/.gitignore"));
    assert!(files.iter().any(|file| file.path == "/.editorconfig"));
    assert!(
        package
            .diff(&AgentPackage::from_zip(&package.to_zip().unwrap()).unwrap())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn instructions_cannot_read_hidden_credentials_from_the_host() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".env"), "TOKEN=secret").unwrap();
    std::fs::write(
        dir.path().join("agent.toml"),
        "name = 'test'\ninstructions_file = '.env'",
    )
    .unwrap();
    let error = AgentPackage::load(dir.path()).unwrap_err().to_string();
    assert!(
        error.contains("instructions_file") && error.contains("credential"),
        "{error}"
    );
}

#[test]
fn empty_directory_trees_have_bounded_nesting() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("agent.toml"),
        "name = 'deep'\ninstructions = 'Hello'",
    )
    .unwrap();
    let mut dir = root.path().join("files");
    for _ in 0..35 {
        dir = dir.join("nested");
    }
    std::fs::create_dir_all(&dir).unwrap();
    assert!(
        AgentPackage::load(root.path())
            .unwrap_err()
            .to_string()
            .contains("nesting")
    );
}

#[test]
fn folder_text_exports_preserve_embedded_skill_frontmatter() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../examples/agent-packages/triage");
    let package = AgentPackage::load(root).unwrap();
    for format in [Format::Markdown, Format::Yaml, Format::Toml, Format::Json] {
        let text = package.to_string(format).unwrap();
        let restored = AgentPackage::parse(&text, format).unwrap();
        assert!(package.diff(&restored).unwrap().is_empty(), "{format:?}");
    }
}

#[test]
fn channel_resource_fanout_is_bounded() {
    let channels = (0..33)
        .map(|i| (format!("chat-{i}"), serde_json::json!({"type":"ag_ui"})))
        .collect::<serde_json::Map<_, _>>();
    let text = serde_json::json!({"name":"bounded","instructions":"Hello","channels":channels})
        .to_string();
    let error = AgentPackage::parse(&text, Format::Json).unwrap_err();
    assert!(error.0.iter().any(|d| d.path == "channels"));
}

#[test]
fn root_layout_preserves_paths_without_collecting_the_project() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("data")).unwrap();
    std::fs::create_dir_all(dir.path().join(".agents/skills/check/scripts")).unwrap();
    std::fs::write(
        dir.path().join("agent.toml"),
        "name = 'root-agent'\ninstructions_file = 'instructions.md'\nfiles = ['data/**']",
    )
    .unwrap();
    std::fs::write(dir.path().join("instructions.md"), "Use the data").unwrap();
    std::fs::write(dir.path().join("data/input.csv"), "value\n42").unwrap();
    std::fs::write(dir.path().join("unselected.txt"), "do not package").unwrap();
    std::fs::write(
        dir.path().join(".agents/skills/check/SKILL.md"),
        "---\nname: check\ndescription: Check data\n---\nCheck the data.",
    )
    .unwrap();
    std::fs::write(
        dir.path().join(".agents/skills/check/scripts/check.py"),
        "print(42)",
    )
    .unwrap();
    let package = AgentPackage::load(dir.path().join("agent.toml")).unwrap();
    let paths: Vec<_> = package
        .files()
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect();
    assert_eq!(
        paths,
        [
            "/data/input.csv",
            "/.agents/skills/check/SKILL.md",
            "/.agents/skills/check/scripts/check.py"
        ]
    );
    let entries = package.folder_entries().unwrap();
    assert!(entries.contains_key("data/input.csv"));
    assert!(entries.contains_key(".agents/skills/check/scripts/check.py"));
    assert!(!entries.contains_key("unselected.txt"));
    assert!(
        !entries
            .keys()
            .any(|p| p.starts_with("files/") || p.starts_with("skills/"))
    );
    let manifest = std::str::from_utf8(&entries["agent.toml"]).unwrap();
    assert!(manifest.contains("files"));
    assert!(!manifest.contains("initial_files"));
    assert!(!manifest.contains("path ="));
    let restored = AgentPackage::from_zip(&package.to_zip().unwrap()).unwrap();
    assert!(package.diff(&restored).unwrap().is_empty());
}

#[test]
fn explicit_empty_files_does_not_discover_legacy_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("files")).unwrap();
    std::fs::write(dir.path().join("files/unselected.txt"), "do not package").unwrap();
    std::fs::write(
        dir.path().join("agent.toml"),
        "name = 'empty'\ninstructions = 'Hi'\nfiles = []",
    )
    .unwrap();
    assert!(
        AgentPackage::load(dir.path())
            .unwrap()
            .files()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn files_alias_and_diff_use_relative_portable_paths() {
    let legacy = AgentPackage::parse(r#"{"name":"test","instructions":"Hi","initial_files":[{"path":"/workspace/data/a","content":"old"}]}"#, Format::Json).unwrap();
    let current = AgentPackage::parse(
        r#"{"name":"test","instructions":"Hi","files":[{"path":"data/a","content":"new"}]}"#,
        Format::Json,
    )
    .unwrap();
    let text = current.to_string(Format::Json).unwrap();
    assert!(text.contains("\"files\""));
    assert!(!text.contains("initial_files"));
    let changes = legacy.diff(&current).unwrap();
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].path, "/files/data~1a");
    assert!(
        AgentPackage::parse(
            r#"{"name":"test","instructions":"Hi","files":[],"initial_files":[]}"#,
            Format::Json
        )
        .is_err()
    );
}

#[test]
fn root_export_preserves_files_colliding_with_authoring_metadata() {
    let package = AgentPackage::parse(r#"{"name":"test","instructions":"Hi","files":[{"path":"agent.toml","content":"runtime config"},{"path":"instructions.md","content":"runtime notes"},{"path":"agent.json","content":"{}"}]}"#, Format::Json).unwrap();
    let entries = package.folder_entries().unwrap();
    assert_eq!(entries["instructions.md"], b"Hi");
    assert!(!entries.contains_key("agent.json"));
    assert!(
        package
            .diff(&AgentPackage::from_zip(&package.to_zip().unwrap()).unwrap())
            .unwrap()
            .is_empty()
    );
}

#[test]
fn ambiguous_skill_roots_require_explicit_selection() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("agent.toml"),
        "name = 'test'\ninstructions = 'Hi'",
    )
    .unwrap();
    for root in ["skills", ".agents/skills"] {
        std::fs::create_dir_all(dir.path().join(root).join("check")).unwrap();
        std::fs::write(
            dir.path().join(root).join("check/SKILL.md"),
            "---\nname: check\ndescription: Check\n---\nCheck.",
        )
        .unwrap();
    }
    assert!(
        AgentPackage::load(dir.path())
            .unwrap_err()
            .to_string()
            .contains("declare skills explicitly")
    );
    std::fs::write(
        dir.path().join("agent.toml"),
        "name = 'test'\ninstructions = 'Hi'\nskills = ['.agents/skills']",
    )
    .unwrap();
    assert_eq!(
        AgentPackage::load(dir.path())
            .unwrap()
            .files()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn virtual_folder_selection_reads_only_referenced_assets() {
    let package = AgentPackage::parse(
        "name = 'test'\ninstructions = 'Hi'\nfiles = ['data/*.csv']",
        Format::Toml,
    )
    .unwrap();
    let selected = package
        .referenced_paths([
            "agent.toml",
            "data/input.csv",
            "data/unselected.bin",
            "unselected.txt",
            ".env",
        ])
        .unwrap();
    assert_eq!(selected.into_iter().collect::<Vec<_>>(), ["data/input.csv"]);
    assert!(AgentPackage::parse(r#"{"name":"test","instructions":"Hi","files":[{"path":"a","content":"x"},{"path":"a/b","content":"y"}]}"#, Format::Json).unwrap_err().to_string().contains("parent file"));
}

#[test]
fn native_globs_do_not_read_unselected_large_siblings() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("data")).unwrap();
    std::fs::write(
        dir.path().join("agent.toml"),
        "name = 'test'\ninstructions = 'Hi'\nfiles = ['data/*.csv']",
    )
    .unwrap();
    std::fs::write(dir.path().join("data/input.csv"), "42").unwrap();
    std::fs::write(
        dir.path().join("data/unselected.bin"),
        vec![0; 1024 * 1024 + 1],
    )
    .unwrap();
    assert_eq!(
        AgentPackage::load(dir.path())
            .unwrap()
            .files()
            .unwrap()
            .len(),
        1
    );
}
