use super::*;
use crate::capabilities::Capability;
use crate::error::Result;
use crate::session_file::{FileInfo, FileStat, GrepMatch, SessionFile};
use crate::typed_id::SessionId;
use everruns_core::session_files::SessionFileSystem;
use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

// ----------------------------------------------------------------------------
// In-memory mock file store (files + directories), keyed by VFS path.
// ----------------------------------------------------------------------------

struct MockFs {
    files: Mutex<HashMap<(SessionId, String), String>>,
    dirs: Mutex<HashSet<(SessionId, String)>>,
}

impl MockFs {
    fn new() -> Self {
        Self {
            files: Mutex::new(HashMap::new()),
            dirs: Mutex::new(HashSet::new()),
        }
    }

    fn add_file(&self, session_id: SessionId, path: &str, content: &str) {
        self.files
            .lock()
            .unwrap()
            .insert((session_id, path.to_string()), content.to_string());
        let mut dir = path.to_string();
        while let Some(idx) = dir.rfind('/') {
            if idx == 0 {
                break;
            }
            dir = dir[..idx].to_string();
            self.dirs.lock().unwrap().insert((session_id, dir.clone()));
        }
    }
}

#[async_trait]
impl SessionFileSystem for MockFs {
    fn is_mount_resolver(&self) -> bool {
        false
    }

    async fn read_file(&self, session_id: SessionId, path: &str) -> Result<Option<SessionFile>> {
        let files = self.files.lock().unwrap();
        Ok(files
            .get(&(session_id, path.to_string()))
            .map(|content| session_file(session_id, path, content)))
    }

    async fn write_file(
        &self,
        session_id: SessionId,
        path: &str,
        content: &str,
        _encoding: &str,
    ) -> Result<SessionFile> {
        self.add_file(session_id, path, content);
        Ok(session_file(session_id, path, content))
    }

    async fn delete_file(
        &self,
        _session_id: SessionId,
        _path: &str,
        _recursive: bool,
    ) -> Result<bool> {
        Ok(false)
    }

    async fn list_directory(&self, session_id: SessionId, path: &str) -> Result<Vec<FileInfo>> {
        let files = self.files.lock().unwrap();
        let dirs = self.dirs.lock().unwrap();
        let prefix = if path.ends_with('/') {
            path.to_string()
        } else {
            format!("{path}/")
        };
        let mut entries = Vec::new();
        let mut seen = HashSet::new();
        for ((sid, file_path), content) in files.iter() {
            if *sid != session_id || !file_path.starts_with(&prefix) {
                continue;
            }
            let remainder = &file_path[prefix.len()..];
            if !remainder.contains('/') {
                entries.push(file_info(
                    session_id,
                    file_path,
                    remainder,
                    false,
                    content.len(),
                ));
            }
        }
        for (sid, dir_path) in dirs.iter() {
            if *sid != session_id || !dir_path.starts_with(&prefix) {
                continue;
            }
            let remainder = &dir_path[prefix.len()..];
            if !remainder.contains('/') && !remainder.is_empty() && seen.insert(dir_path.clone()) {
                entries.push(file_info(session_id, dir_path, remainder, true, 0));
            }
        }
        Ok(entries)
    }

    async fn stat_file(&self, _session_id: SessionId, _path: &str) -> Result<Option<FileStat>> {
        Ok(None)
    }

    async fn grep_files(
        &self,
        _session_id: SessionId,
        _pattern: &str,
        _path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        Ok(vec![])
    }

    async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo> {
        self.dirs
            .lock()
            .unwrap()
            .insert((session_id, path.to_string()));
        let name = path.rsplit('/').next().unwrap_or(path);
        Ok(file_info(session_id, path, name, true, 0))
    }
}

fn session_file(session_id: SessionId, path: &str, content: &str) -> SessionFile {
    SessionFile {
        id: uuid::Uuid::new_v4(),
        session_id: session_id.into(),
        path: path.to_string(),
        name: path.split('/').next_back().unwrap_or("").to_string(),
        is_directory: false,
        is_readonly: false,
        content: Some(content.to_string()),
        encoding: "text".to_string(),
        size_bytes: content.len() as i64,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

fn file_info(session_id: SessionId, path: &str, name: &str, is_dir: bool, size: usize) -> FileInfo {
    FileInfo {
        id: uuid::Uuid::new_v4(),
        session_id: session_id.into(),
        path: path.to_string(),
        name: name.to_string(),
        is_directory: is_dir,
        is_readonly: false,
        size_bytes: size as i64,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

fn valid_skill_md(name: &str, desc: &str) -> String {
    format!("---\nname: {name}\ndescription: {desc}\n---\n\n# Instructions\nDo {name}.")
}

fn ctx(fs: Arc<MockFs>, session_id: SessionId) -> ToolContext {
    ToolContext::with_file_store(session_id, fs)
}

// Two scopes, with `workspace` shadowing `global`.
fn two_scope_config() -> SkillsConfig {
    SkillsConfig {
        scopes: vec![
            SkillScope::new("workspace", "/.agents/skills", true),
            SkillScope::new("global", "/.global/skills", true),
        ],
        manage_tools: true,
        ..Default::default()
    }
}

// ----------------------------------------------------------------------------
// Capability metadata / tool gating
// ----------------------------------------------------------------------------

#[test]
fn default_config_exposes_two_tools() {
    let cap = ScopedSkillsCapability::new(SkillsConfig::default());
    let tools = cap.tools();
    assert_eq!(tools.len(), 2);
    assert_eq!(tools[0].name(), "list_skills");
    assert_eq!(tools[1].name(), "activate_skill");
    assert_eq!(cap.tool_definitions().len(), 2);
    assert_eq!(cap.id(), "skills");
    assert_eq!(cap.dependencies(), vec!["session_file_system"]);
}

#[test]
fn manage_tools_adds_read_and_write() {
    let cap = ScopedSkillsCapability::new(two_scope_config());
    let tools = cap.tools();
    let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
    assert_eq!(tools.len(), 4);
    assert!(names.contains(&"read_skill"));
    assert!(names.contains(&"write_skill"));
    assert_eq!(cap.tool_definitions().len(), 4);
}

// ----------------------------------------------------------------------------
// Discovery / list_skills
// ----------------------------------------------------------------------------

#[tokio::test]
async fn list_skills_default_scope_uses_workspace_display_path() {
    let fs = Arc::new(MockFs::new());
    let sid = SessionId::new();
    fs.add_file(
        sid,
        "/.agents/skills/pdf-tool/SKILL.md",
        &valid_skill_md("pdf-tool", "Extract text from PDFs"),
    );
    let cap = ScopedSkillsCapability::new(SkillsConfig::default());
    let tool = &cap.tools()[0];
    let result = tool.execute_with_context(json!({}), &ctx(fs, sid)).await;
    match result {
        ToolExecutionResult::Success(val) => {
            assert_eq!(val["count"], 1);
            let s = &val["skills"][0];
            assert_eq!(s["name"], "pdf-tool");
            assert_eq!(s["scope"], "workspace");
            assert_eq!(s["path"], "/workspace/.agents/skills/pdf-tool/SKILL.md");
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn list_skills_merges_scopes_with_precedence() {
    let fs = Arc::new(MockFs::new());
    let sid = SessionId::new();
    // Same dir name in both scopes — workspace must win.
    fs.add_file(
        sid,
        "/.agents/skills/shared/SKILL.md",
        &valid_skill_md("shared", "workspace copy"),
    );
    fs.add_file(
        sid,
        "/.global/skills/shared/SKILL.md",
        &valid_skill_md("shared", "global copy"),
    );
    fs.add_file(
        sid,
        "/.global/skills/only-global/SKILL.md",
        &valid_skill_md("only-global", "global only"),
    );

    let cap = ScopedSkillsCapability::new(two_scope_config());
    let list = &cap.tools()[0];
    let result = list.execute_with_context(json!({}), &ctx(fs, sid)).await;
    let val = match result {
        ToolExecutionResult::Success(v) => v,
        other => panic!("expected success, got {other:?}"),
    };
    assert_eq!(val["count"], 2);
    let skills = val["skills"].as_array().unwrap();
    let shared = skills.iter().find(|s| s["name"] == "shared").unwrap();
    assert_eq!(shared["scope"], "workspace");
    assert_eq!(shared["description"], "workspace copy");
    let og = skills.iter().find(|s| s["name"] == "only-global").unwrap();
    assert_eq!(og["scope"], "global");
}

#[tokio::test]
async fn list_skills_is_not_capped_by_prompt_scan_limit() {
    // Regression: discovery is shared with the system prompt (which caps at
    // MAX_SKILLS_SCAN_PER_SCOPE=64), but list_skills must return the full set.
    let fs = Arc::new(MockFs::new());
    let sid = SessionId::new();
    for i in 0..70 {
        let name = format!("skill-{i:03}");
        fs.add_file(
            sid,
            &format!("/.agents/skills/{name}/SKILL.md"),
            &valid_skill_md(&name, "many"),
        );
    }
    let cap = ScopedSkillsCapability::new(SkillsConfig::default());
    let list = &cap.tools()[0];
    let result = list.execute_with_context(json!({}), &ctx(fs, sid)).await;
    match result {
        ToolExecutionResult::Success(val) => assert_eq!(val["count"], 70),
        other => panic!("expected success, got {other:?}"),
    }
}

// ----------------------------------------------------------------------------
// activate_skill
// ----------------------------------------------------------------------------

#[tokio::test]
async fn activate_skill_default_resolver_substitutes_vfs_skill_dir() {
    let fs = Arc::new(MockFs::new());
    let sid = SessionId::new();
    fs.add_file(
        sid,
        "/.agents/skills/echo/SKILL.md",
        "---\nname: echo\ndescription: echo dir\n---\nDir is ${SKILL_DIR}.",
    );
    let cap = ScopedSkillsCapability::new(SkillsConfig::default());
    let activate = &cap.tools()[1];
    let result = activate
        .execute_with_context(json!({ "name": "echo" }), &ctx(fs, sid))
        .await;
    match result {
        ToolExecutionResult::Success(val) => {
            assert_eq!(val["skill"], "echo");
            assert_eq!(val["scope"], "workspace");
            let instructions = val["instructions"].as_str().unwrap();
            assert!(instructions.contains("Dir is /.agents/skills/echo."));
            assert!(instructions.contains("<skill name=\"echo\">"));
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn activate_skill_custom_resolver_overrides_skill_dir() {
    struct HostResolver;
    impl SkillDirResolver for HostResolver {
        fn skill_dir(&self, scope: &SkillScope, name: &str) -> String {
            format!("/host/{}/{name}", scope.label)
        }
    }
    let fs = Arc::new(MockFs::new());
    let sid = SessionId::new();
    fs.add_file(
        sid,
        "/.agents/skills/echo/SKILL.md",
        "---\nname: echo\ndescription: echo dir\n---\nDir is ${SKILL_DIR}.",
    );
    let config = SkillsConfig {
        resolver: Arc::new(HostResolver),
        ..Default::default()
    };
    let cap = ScopedSkillsCapability::new(config);
    let activate = &cap.tools()[1];
    let result = activate
        .execute_with_context(json!({ "name": "echo" }), &ctx(fs, sid))
        .await;
    match result {
        ToolExecutionResult::Success(val) => {
            let instructions = val["instructions"].as_str().unwrap();
            assert!(instructions.contains("Dir is /host/workspace/echo."));
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn activate_skill_rejects_path_traversal() {
    let fs = Arc::new(MockFs::new());
    let sid = SessionId::new();
    let cap = ScopedSkillsCapability::new(SkillsConfig::default());
    let activate = &cap.tools()[1];
    let result = activate
        .execute_with_context(json!({ "name": "../etc/passwd" }), &ctx(fs, sid))
        .await;
    assert!(matches!(result, ToolExecutionResult::ToolError(_)));
}

#[tokio::test]
async fn activate_skill_not_found() {
    let fs = Arc::new(MockFs::new());
    let sid = SessionId::new();
    let cap = ScopedSkillsCapability::new(SkillsConfig::default());
    let activate = &cap.tools()[1];
    let result = activate
        .execute_with_context(json!({ "name": "nope" }), &ctx(fs, sid))
        .await;
    match result {
        ToolExecutionResult::ToolError(msg) => assert!(msg.contains("not found")),
        other => panic!("expected error, got {other:?}"),
    }
}

#[tokio::test]
async fn activate_skill_missing_name_returns_actionable_error() {
    let fs = Arc::new(MockFs::new());
    let sid = SessionId::new();
    let cap = ScopedSkillsCapability::new(SkillsConfig::default());
    let activate = &cap.tools()[1];
    let result = activate
        .execute_with_context(json!({}), &ctx(fs, sid))
        .await;
    match result {
        ToolExecutionResult::ToolError(msg) => {
            // Names the missing field, shows an example JSON shape, and points
            // at list_skills so a malformed call can self-repair.
            assert!(msg.contains("name"), "should name the missing field: {msg}");
            assert!(
                msg.contains("{\"name\":\"ship\"}"),
                "should include an example JSON shape: {msg}"
            );
            assert!(
                msg.contains("list_skills"),
                "should mention list_skills: {msg}"
            );
        }
        other => panic!("expected error, got {other:?}"),
    }
}

// ----------------------------------------------------------------------------
// write_skill / read_skill
// ----------------------------------------------------------------------------

#[tokio::test]
async fn write_then_discover_and_read_skill() {
    let fs = Arc::new(MockFs::new());
    let sid = SessionId::new();
    let cap = ScopedSkillsCapability::new(two_scope_config());
    let tools = cap.tools();
    let write = tools.iter().find(|t| t.name() == "write_skill").unwrap();
    let read = tools.iter().find(|t| t.name() == "read_skill").unwrap();
    let list = tools.iter().find(|t| t.name() == "list_skills").unwrap();

    let result = write
        .execute_with_context(
            json!({
                "scope": "global",
                "name": "greeter",
                "skill_md": valid_skill_md("greeter", "say hi"),
                "files": { "data/notes.txt": "hello" }
            }),
            &ctx(fs.clone(), sid),
        )
        .await;
    match result {
        ToolExecutionResult::Success(val) => {
            assert_eq!(val["ok"], true);
            assert_eq!(val["scope"], "global");
            assert_eq!(val["files_written"], 2);
        }
        other => panic!("expected success, got {other:?}"),
    }

    // Discoverable immediately.
    let listed = list
        .execute_with_context(json!({}), &ctx(fs.clone(), sid))
        .await;
    let val = match listed {
        ToolExecutionResult::Success(v) => v,
        other => panic!("expected success, got {other:?}"),
    };
    assert_eq!(val["count"], 1);
    assert_eq!(val["skills"][0]["name"], "greeter");

    // read_skill returns SKILL.md + the bundled file manifest.
    let read_res = read
        .execute_with_context(json!({ "name": "greeter" }), &ctx(fs, sid))
        .await;
    match read_res {
        ToolExecutionResult::Success(val) => {
            assert_eq!(val["scope"], "global");
            assert!(val["skill_md"].as_str().unwrap().contains("name: greeter"));
            let files = val["files"].as_array().unwrap();
            assert_eq!(files.len(), 1);
            assert_eq!(files[0], "data/notes.txt");
        }
        other => panic!("expected success, got {other:?}"),
    }
}

#[tokio::test]
async fn write_skill_rejects_readonly_scope() {
    let fs = Arc::new(MockFs::new());
    let sid = SessionId::new();
    let config = SkillsConfig {
        scopes: vec![
            SkillScope::new("workspace", "/.agents/skills", true),
            SkillScope::new("system", "/.system/skills", false),
        ],
        manage_tools: true,
        ..Default::default()
    };
    let cap = ScopedSkillsCapability::new(config);
    let write = cap
        .tools()
        .into_iter()
        .find(|t| t.name() == "write_skill")
        .unwrap();
    let result = write
        .execute_with_context(
            json!({
                "scope": "system",
                "name": "greeter",
                "skill_md": valid_skill_md("greeter", "say hi"),
            }),
            &ctx(fs, sid),
        )
        .await;
    match result {
        ToolExecutionResult::ToolError(msg) => assert!(msg.contains("read-only")),
        other => panic!("expected error, got {other:?}"),
    }
}

#[tokio::test]
async fn write_skill_name_must_match_frontmatter() {
    let fs = Arc::new(MockFs::new());
    let sid = SessionId::new();
    let cap = ScopedSkillsCapability::new(two_scope_config());
    let write = cap
        .tools()
        .into_iter()
        .find(|t| t.name() == "write_skill")
        .unwrap();
    let result = write
        .execute_with_context(
            json!({
                "name": "greeter",
                "skill_md": valid_skill_md("different", "mismatch"),
            }),
            &ctx(fs, sid),
        )
        .await;
    match result {
        ToolExecutionResult::ToolError(msg) => assert!(msg.contains("must match")),
        other => panic!("expected error, got {other:?}"),
    }
}

#[tokio::test]
async fn write_skill_rejects_skill_md_in_files() {
    let fs = Arc::new(MockFs::new());
    let sid = SessionId::new();
    let cap = ScopedSkillsCapability::new(two_scope_config());
    let write = cap
        .tools()
        .into_iter()
        .find(|t| t.name() == "write_skill")
        .unwrap();
    let result = write
        .execute_with_context(
            json!({
                "name": "greeter",
                "skill_md": valid_skill_md("greeter", "hi"),
                "files": { "SKILL.md": "nope" }
            }),
            &ctx(fs, sid),
        )
        .await;
    assert!(matches!(result, ToolExecutionResult::ToolError(_)));
}
