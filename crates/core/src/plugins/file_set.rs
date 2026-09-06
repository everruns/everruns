// Plugin file set: in-memory representation of a plugin directory.
//
// PluginFileSet walks a directory on disk and captures its contents as a map
// of relative path → bytes, subject to size and count limits mirroring those
// of the declarative capability system.

use std::collections::BTreeMap;
use std::path::{Component, Path};

use super::manifest::{
    AGENT_PLUGINS_V1_MANIFEST_SCHEMA, PluginManifest, parse_agent_plugins_v1_manifest,
};

// Package ingestion limits. Compiled text contributions still pass the
// declarative capability's stricter per-component validation.
/// Maximum number of files in a plugin directory.
pub const MAX_PLUGIN_FILES: usize = 256;
/// Maximum bytes per individual file.
pub const MAX_PLUGIN_FILE_BYTES: usize = 128 * 1024;
/// Maximum total bytes across all files.
pub const MAX_PLUGIN_TOTAL_BYTES: usize = 4 * 1024 * 1024; // 4 MB

/// Manifest discovery priority order.
const MANIFEST_PATHS: &[&str] = &[
    ".claude-plugin/plugin.json",
    ".codex-plugin/plugin.json",
    ".cursor-plugin/plugin.json",
];

/// In-memory representation of a loaded plugin directory.
///
/// Relative path → raw bytes for every file within the plugin directory.
/// The map is a `BTreeMap` so iteration order is deterministic (useful for
/// tests and for reproducing compilation results across runs).
#[derive(Debug, Clone)]
pub struct PluginFileSet {
    /// All files, keyed by relative path (forward-slash separated, no leading slash).
    pub files: BTreeMap<String, Vec<u8>>,
    /// The directory name (used for manifest synthesis when no manifest is found).
    pub dir_name: String,
}

impl PluginFileSet {
    /// Build a `PluginFileSet` from an in-memory map of relative path → bytes.
    ///
    /// Applies the same per-file, total-size, and count limits as `from_dir`.
    /// Rejects any path that contains `..` components or an absolute leading `/`.
    /// This is the seam for tarball extraction and tests — no disk access required.
    pub fn from_map(
        dir_name: impl Into<String>,
        files: BTreeMap<String, Vec<u8>>,
    ) -> Result<Self, String> {
        let mut total_bytes: usize = 0;
        if files.len() > MAX_PLUGIN_FILES {
            return Err(format!(
                "plugin contains {} files, exceeding the {MAX_PLUGIN_FILES}-file limit",
                files.len()
            ));
        }
        for (path, bytes) in &files {
            // Reject absolute paths and traversals.
            if path.starts_with('/') {
                return Err(format!("plugin file path '{path}' must be relative"));
            }
            for component in std::path::Path::new(path).components() {
                if component == Component::ParentDir {
                    return Err(format!(
                        "path traversal detected in plugin file map: '{path}'"
                    ));
                }
            }
            let file_size = bytes.len();
            if file_size > MAX_PLUGIN_FILE_BYTES {
                return Err(format!(
                    "plugin file '{path}' is {file_size} bytes, exceeding the {MAX_PLUGIN_FILE_BYTES}-byte limit"
                ));
            }
            total_bytes += file_size;
            if total_bytes > MAX_PLUGIN_TOTAL_BYTES {
                return Err(format!(
                    "plugin total size exceeds {MAX_PLUGIN_TOTAL_BYTES} bytes"
                ));
            }
        }
        Ok(Self {
            files,
            dir_name: dir_name.into(),
        })
    }

    /// Load a plugin directory from disk.
    ///
    /// - Rejects `..` components and all symlinks (cycle/escape defense).
    /// - Skips files larger than `MAX_PLUGIN_FILE_BYTES`.
    /// - Fails if more than `MAX_PLUGIN_FILES` files are found.
    /// - Fails if total bytes exceed `MAX_PLUGIN_TOTAL_BYTES`.
    pub fn from_dir(path: &Path) -> Result<Self, String> {
        let canonical_root = path.canonicalize().map_err(|e| {
            format!(
                "cannot canonicalize plugin directory {}: {}",
                path.display(),
                e
            )
        })?;

        let dir_name = canonical_root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("plugin")
            .to_string();

        let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut total_bytes: usize = 0;

        collect_dir(
            &canonical_root,
            &canonical_root,
            &mut files,
            &mut total_bytes,
        )?;

        Ok(Self { files, dir_name })
    }

    /// Resolve the plugin manifest.
    ///
    /// A canonical root `plugin.json` takes precedence when it declares the
    /// Agent Plugins schema. Otherwise discovery falls back to the legacy
    /// `.claude-plugin`, `.codex-plugin`, and `.cursor-plugin` manifests. If no
    /// manifest is found, a minimal one is synthesized from the directory name.
    pub fn manifest(&self) -> Result<(PluginManifest, Vec<String>), String> {
        if let Some(bytes) = self.files.get("plugin.json") {
            let text = std::str::from_utf8(bytes)
                .map_err(|_| "plugin.json is not valid UTF-8".to_string())?;
            let schema = serde_json::from_str::<serde_json::Value>(text)
                .ok()
                .and_then(|value| value.get("$schema")?.as_str().map(str::to_string));
            if schema.as_deref() == Some(AGENT_PLUGINS_V1_MANIFEST_SCHEMA)
                || schema
                    .as_deref()
                    .is_some_and(|schema| schema.starts_with("https://agent-plugins.org/schemas/"))
            {
                return parse_agent_plugins_v1_manifest(text);
            }
            if !MANIFEST_PATHS
                .iter()
                .any(|manifest_path| self.files.contains_key(*manifest_path))
            {
                return parse_agent_plugins_v1_manifest(text);
            }
        }

        for manifest_path in MANIFEST_PATHS {
            if let Some(bytes) = self.files.get(*manifest_path) {
                let text = std::str::from_utf8(bytes)
                    .map_err(|_| format!("{manifest_path} is not valid UTF-8"))?;
                let manifest: PluginManifest = serde_json::from_str(text)
                    .map_err(|e| format!("failed to parse {manifest_path}: {e}"))?;
                let mut warnings = Vec::new();
                for key in manifest.extra.keys() {
                    warnings.push(format!(
                        "plugin manifest: unrecognized field '{key}' will be ignored"
                    ));
                }
                if self.files.contains_key("plugin.json") {
                    warnings.push(
                        "root plugin.json does not declare an Agent Plugins schema and was ignored"
                            .to_string(),
                    );
                }
                return Ok((manifest, warnings));
            }
        }

        // Synthesize a minimal manifest from the directory name.
        let name = dir_name_to_plugin_name(&self.dir_name);
        Ok((
            PluginManifest {
                schema: None,
                name,
                display_name: None,
                version: None,
                description: None,
                author: None,
                homepage: None,
                repository: None,
                license: None,
                keywords: Vec::new(),
                icon: None,
                extensions: Default::default(),
                skills: None,
                commands: None,
                agents: None,
                mcp_servers: None,
                extra: Default::default(),
            },
            vec!["no plugin.json manifest found; name derived from directory name".to_string()],
        ))
    }

    /// Retrieve a file's content as a UTF-8 string, or `None` if not found or binary.
    pub fn text_file(&self, path: &str) -> Option<String> {
        let bytes = self.files.get(path)?;
        String::from_utf8(bytes.clone()).ok()
    }

    /// List relative paths that are direct children of `dir_prefix/`.
    /// Returns `(relative_within_dir, full_relative_path)`.
    pub fn list_dir<'a>(&'a self, dir_prefix: &str) -> Vec<(&'a str, &'a str)> {
        let prefix = if dir_prefix.ends_with('/') {
            dir_prefix.to_string()
        } else {
            format!("{dir_prefix}/")
        };
        self.files
            .keys()
            .filter_map(|k| {
                let rest = k.strip_prefix(&prefix)?;
                if rest.is_empty() || rest.contains('/') {
                    None
                } else {
                    Some((rest, k.as_str()))
                }
            })
            .collect()
    }

    /// List relative paths for all files under `dir_prefix/` (recursively).
    pub fn list_dir_recursive<'a>(&'a self, dir_prefix: &str) -> Vec<&'a str> {
        let prefix = if dir_prefix.ends_with('/') {
            dir_prefix.to_string()
        } else {
            format!("{dir_prefix}/")
        };
        self.files
            .keys()
            .filter(|k| k.starts_with(&prefix))
            .map(|k| k.as_str())
            .collect()
    }
}

/// Convert a filesystem directory name into a valid plugin name (kebab-case).
fn dir_name_to_plugin_name(name: &str) -> String {
    let lower = name.to_lowercase();
    // Replace anything that isn't [a-z0-9-] with a hyphen, then trim leading/trailing hyphens.
    let result: String = lower
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() {
                c
            } else {
                '-'
            }
        })
        .collect();
    // Collapse runs of hyphens and strip leading/trailing hyphens.
    let mut out = String::new();
    let mut prev_was_hyphen = true; // start true so leading hyphens are stripped
    for ch in result.chars() {
        if ch == '-' {
            if !prev_was_hyphen {
                out.push(ch);
            }
            prev_was_hyphen = true;
        } else {
            out.push(ch);
            prev_was_hyphen = false;
        }
    }
    // Strip trailing hyphen.
    let out = out.trim_end_matches('-');
    if out.is_empty() {
        "plugin".to_string()
    } else {
        out.to_string()
    }
}

/// Recursively collect files from `current` into `files`.
fn collect_dir(
    root: &Path,
    current: &Path,
    files: &mut BTreeMap<String, Vec<u8>>,
    total_bytes: &mut usize,
) -> Result<(), String> {
    let entries = std::fs::read_dir(current)
        .map_err(|e| format!("cannot read directory {}: {}", current.display(), e))?;

    for entry_result in entries {
        let entry = entry_result.map_err(|e| {
            format!(
                "error reading directory entry in {}: {}",
                current.display(),
                e
            )
        })?;
        let entry_path = entry.path();

        // Reject symlinks.
        let metadata = entry_path
            .symlink_metadata()
            .map_err(|e| format!("cannot stat {}: {}", entry_path.display(), e))?;
        if metadata.file_type().is_symlink() {
            // Reject symlinks outright: even an in-root link can form a
            // directory cycle (unbounded traversal), and tarball extraction
            // already skips link entries — keep both ingestion paths
            // consistent.
            return Err(format!(
                "symlink {} is not allowed in a plugin directory",
                entry_path.display()
            ));
        }

        // Build a relative path (forward-slash, no leading slash).
        let rel = entry_path.strip_prefix(root).map_err(|_| {
            format!(
                "path {} is not under root {}",
                entry_path.display(),
                root.display()
            )
        })?;

        // Validate that no path component is `..`.
        for component in rel.components() {
            if component == Component::ParentDir {
                return Err(format!(
                    "path traversal detected in plugin directory: {}",
                    entry_path.display()
                ));
            }
        }

        let rel_str = rel.to_string_lossy().replace('\\', "/");

        if metadata.is_dir() {
            collect_dir(root, &entry_path, files, total_bytes)?;
        } else {
            // It's a file.
            let file_size = metadata.len() as usize;
            if file_size > MAX_PLUGIN_FILE_BYTES {
                // Skip oversized files with a note (caller decides whether to warn).
                // We signal this by recording an empty entry under a sentinel path.
                // Instead, return an error so compile_plugin can decide.
                return Err(format!(
                    "plugin file '{rel_str}' is {file_size} bytes, exceeding the {MAX_PLUGIN_FILE_BYTES}-byte limit"
                ));
            }
            *total_bytes += file_size;
            if *total_bytes > MAX_PLUGIN_TOTAL_BYTES {
                return Err(format!(
                    "plugin directory total size exceeds {MAX_PLUGIN_TOTAL_BYTES} bytes"
                ));
            }
            if files.len() >= MAX_PLUGIN_FILES {
                return Err(format!(
                    "plugin directory contains more than {MAX_PLUGIN_FILES} files"
                ));
            }
            let content = std::fs::read(&entry_path)
                .map_err(|e| format!("cannot read {}: {}", entry_path.display(), e))?;
            files.insert(rel_str, content);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file_set(files: &[(&str, &[u8])]) -> PluginFileSet {
        PluginFileSet::from_map(
            "test",
            files
                .iter()
                .map(|(p, b)| (p.to_string(), b.to_vec()))
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn missing_manifest_synthesizes_normalized_name_and_warning() {
        for (directory, expected) in [
            ("microsoft-docs", "microsoft-docs"),
            ("MyPlugin", "myplugin"),
            ("my_plugin", "my-plugin"),
            ("---test---", "test"),
            ("my  plugin", "my-plugin"),
            ("---💡---", "plugin"),
            ("", "plugin"),
        ] {
            let fs = PluginFileSet::from_map(directory, BTreeMap::new()).unwrap();
            let (manifest, warnings) = fs.manifest().unwrap();
            assert_eq!(
                serde_json::to_value(manifest).unwrap(),
                serde_json::json!({"name":expected})
            );
            assert_eq!(
                warnings,
                ["no plugin.json manifest found; name derived from directory name"]
            );
        }
    }

    #[test]
    fn fixture_load_preserves_all_files_and_discovers_legacy_manifest() {
        let fixture = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testdata/plugins/microsoft-docs"
        ));
        let fs = PluginFileSet::from_dir(fixture).unwrap();
        assert_eq!(fs.dir_name, "microsoft-docs");
        assert_eq!(
            fs.files.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                ".claude-plugin/plugin.json",
                ".mcp.json",
                "agents/docs-researcher.md",
                "assets/icon.svg",
                "commands/ms-docs.md",
                "skills/microsoft-docs/SKILL.md"
            ]
        );
        assert!(fs.files.values().all(|bytes| !bytes.is_empty()));
        let (manifest, warnings) = fs.manifest().unwrap();
        assert_eq!(manifest.name, "microsoft-docs");
        assert_eq!(manifest.display_name.as_deref(), Some("Microsoft Docs"));
        assert_eq!(manifest.version.as_deref(), Some("0.1.0"));
        assert_eq!(manifest.icon.as_deref(), Some("./assets/icon.svg"));
        assert_eq!(
            warnings,
            ["plugin manifest: unrecognized field 'interface' will be ignored"]
        );
    }

    #[test]
    fn manifest_priority_and_schema_failures_do_not_silently_fall_back() {
        let mut fs = file_set(&[
            ("plugin.json", br#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"portable-plugin","futureField":true}"#),
            (".claude-plugin/plugin.json", br#"{"name":"claude"}"#),
            (".codex-plugin/plugin.json", br#"{"name":"codex"}"#),
            (".cursor-plugin/plugin.json", br#"{"name":"cursor"}"#),
        ]);
        let (manifest, warnings) = fs.manifest().unwrap();
        assert_eq!(manifest.name, "portable-plugin");
        assert!(manifest.is_agent_plugins_v1());
        assert!(manifest.extra.is_empty());
        assert_eq!(
            warnings,
            ["plugin.json: unrecognized field 'futureField' was ignored"]
        );
        fs.files.insert("plugin.json".into(), br#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"portable-plugin","author":{"name":"Acme","unexpected":true}}"#.to_vec());
        assert!(
            fs.manifest()
                .unwrap_err()
                .starts_with("invalid plugin.json: /author:")
        );
        fs.files.insert("plugin.json".into(), br#"{"$schema":"https://agent-plugins.org/schemas/2.0.0/plugin.schema.json","name":"future"}"#.to_vec());
        assert_eq!(
            fs.manifest().unwrap_err(),
            "unsupported Agent Plugins schema 'https://agent-plugins.org/schemas/2.0.0/plugin.schema.json'; supported schema is https://agent-plugins.org/schemas/1.0.0/plugin.schema.json"
        );
        fs.files.insert(
            "plugin.json".into(),
            br#"{"name":"unrelated-package"}"#.to_vec(),
        );
        let (manifest, warnings) = fs.manifest().unwrap();
        assert_eq!(manifest.name, "claude");
        assert!(!manifest.is_agent_plugins_v1());
        assert_eq!(
            warnings,
            ["root plugin.json does not declare an Agent Plugins schema and was ignored"]
        );
        fs.files.remove("plugin.json");
        for (path, name) in [
            (".claude-plugin/plugin.json", "claude"),
            (".codex-plugin/plugin.json", "codex"),
            (".cursor-plugin/plugin.json", "cursor"),
        ] {
            let (manifest, warnings) = fs.manifest().unwrap();
            assert_eq!(manifest.name, name);
            assert!(warnings.is_empty());
            fs.files.insert(path.into(), vec![0xff]);
            assert_eq!(
                fs.manifest().unwrap_err(),
                format!("{path} is not valid UTF-8")
            );
            fs.files.remove(path);
        }
        fs.files.insert("plugin.json".into(), vec![0xff]);
        assert_eq!(fs.manifest().unwrap_err(), "plugin.json is not valid UTF-8");
    }

    #[test]
    fn map_paths_and_text_listing_preserve_content_and_directory_boundaries() {
        for path in ["/absolute", "../outside", "a/../../outside"] {
            let error = PluginFileSet::from_map("test", BTreeMap::from([(path.into(), vec![])]))
                .unwrap_err();
            assert!(error.contains(path), "{error}");
            assert!(
                error.contains("relative") || error.contains("path traversal"),
                "{error}"
            );
        }
        let fs = file_set(&[
            ("skills/z.md", b"last\r\n"),
            ("skills/a.md", "first é".as_bytes()),
            ("skills/sub/b.bin", &[0xff]),
            ("skills-extra/no.md", b"no"),
        ]);
        assert_eq!(fs.text_file("skills/a.md").as_deref(), Some("first é"));
        assert_eq!(fs.text_file("skills/z.md").as_deref(), Some("last\r\n"));
        assert_eq!(fs.text_file("missing"), None);
        assert_eq!(fs.text_file("skills/sub/b.bin"), None);
        for prefix in ["skills", "skills/"] {
            assert_eq!(
                fs.list_dir(prefix),
                [("a.md", "skills/a.md"), ("z.md", "skills/z.md")]
            );
            assert_eq!(
                fs.list_dir_recursive(prefix),
                ["skills/a.md", "skills/sub/b.bin", "skills/z.md"]
            );
        }
        assert!(fs.list_dir("missing").is_empty());
        assert!(fs.list_dir_recursive("missing").is_empty());
    }

    #[test]
    fn map_and_disk_enforce_literal_count_file_and_total_size_boundaries() {
        for (count, bytes, extra, expected_error) in [
            (256, 0, false, None),
            (257, 0, false, Some("256")),
            (1, 131072, false, None),
            (1, 131073, false, Some("131072-byte limit")),
            (32, 131072, false, None),
            (32, 131072, true, Some("4194304 bytes")),
        ] {
            let mut files: BTreeMap<String, Vec<u8>> = (0..count)
                .map(|i| (format!("file-{i:03}"), vec![b'x'; bytes]))
                .collect();
            if extra {
                files.insert("extra".into(), vec![b'y']);
            }
            let tmp = tempfile::tempdir().unwrap();
            for (path, content) in &files {
                std::fs::write(tmp.path().join(path), content).unwrap();
            }
            for result in [
                PluginFileSet::from_map("test", files.clone()),
                PluginFileSet::from_dir(tmp.path()),
            ] {
                match expected_error {
                    Some(message) => assert!(result.unwrap_err().contains(message)),
                    None => assert_eq!(result.unwrap().files, files),
                }
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn disk_rejects_file_links_directory_cycles_and_escapes() {
        for target in ["inside", "cycle", "outside", "missing"] {
            let tmp = tempfile::tempdir().unwrap();
            let plugin = tmp.path().join("plugin");
            std::fs::create_dir(&plugin).unwrap();
            std::fs::write(plugin.join("README.md"), b"content").unwrap();
            std::fs::write(tmp.path().join("outside"), b"secret").unwrap();
            let destination = match target {
                "inside" => plugin.join("README.md"),
                "cycle" => plugin.clone(),
                other => tmp.path().join(other),
            };
            std::os::unix::fs::symlink(destination, plugin.join("link")).unwrap();
            assert_eq!(
                PluginFileSet::from_dir(&plugin).unwrap_err(),
                format!(
                    "symlink {} is not allowed in a plugin directory",
                    plugin.canonicalize().unwrap().join("link").display()
                )
            );
        }
    }
}
