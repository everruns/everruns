// Plugin file set: in-memory representation of a plugin directory.
//
// PluginFileSet walks a directory on disk and captures its contents as a map
// of relative path → bytes, subject to size and count limits mirroring those
// of the declarative capability system.

use std::collections::BTreeMap;
use std::path::{Component, Path};

use super::manifest::PluginManifest;

// Size / count limits (mirror declarative capability limits).
/// Maximum number of files in a plugin directory.
pub const MAX_PLUGIN_FILES: usize = 256;
/// Maximum bytes per individual file.
pub const MAX_PLUGIN_FILE_BYTES: usize = 64 * 1024;
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
    /// Load a plugin directory from disk.
    ///
    /// - Rejects `..` components and symlinks that escape the root.
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
    /// Discovery order: `.claude-plugin/plugin.json`, then `.codex-plugin/plugin.json`,
    /// then `.cursor-plugin/plugin.json`. If none is found, a minimal manifest
    /// is synthesized from the directory name (Claude Code parity).
    pub fn manifest(&self) -> Result<(PluginManifest, Vec<String>), String> {
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
                return Ok((manifest, warnings));
            }
        }

        // Synthesize a minimal manifest from the directory name.
        let name = dir_name_to_plugin_name(&self.dir_name);
        Ok((
            PluginManifest {
                name,
                display_name: None,
                version: None,
                description: None,
                author: None,
                homepage: None,
                repository: None,
                license: None,
                keywords: Vec::new(),
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
            // Verify the symlink target stays within root.
            let resolved = entry_path
                .canonicalize()
                .map_err(|e| format!("cannot resolve symlink {}: {}", entry_path.display(), e))?;
            if !resolved.starts_with(root) {
                return Err(format!(
                    "symlink {} escapes plugin root — rejected for security",
                    entry_path.display()
                ));
            }
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

        if metadata.is_dir() || metadata.file_type().is_symlink() && entry_path.is_dir() {
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

    #[test]
    fn dir_name_to_plugin_name_simple() {
        assert_eq!(dir_name_to_plugin_name("microsoft-docs"), "microsoft-docs");
        assert_eq!(dir_name_to_plugin_name("MyPlugin"), "myplugin");
        assert_eq!(dir_name_to_plugin_name("my_plugin"), "my-plugin");
        assert_eq!(dir_name_to_plugin_name("---test---"), "test");
        assert_eq!(dir_name_to_plugin_name("my  plugin"), "my-plugin");
    }

    #[test]
    fn plugin_file_set_from_fixture() {
        let fixture = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testdata/plugins/microsoft-docs"
        ));
        let fs = PluginFileSet::from_dir(fixture).expect("should load microsoft-docs fixture");
        assert!(fs.files.contains_key(".claude-plugin/plugin.json"));
        assert!(fs.files.contains_key(".mcp.json"));
        assert!(fs.files.contains_key("agents/docs-researcher.md"));
        assert!(fs.files.contains_key("skills/microsoft-docs/SKILL.md"));
        assert!(fs.files.contains_key("commands/ms-docs.md"));
    }

    #[test]
    fn manifest_discovery_from_fixture() {
        let fixture = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../testdata/plugins/microsoft-docs"
        ));
        let fs = PluginFileSet::from_dir(fixture).unwrap();
        let (manifest, warnings) = fs.manifest().unwrap();
        assert_eq!(manifest.name, "microsoft-docs");
        assert!(
            warnings.iter().any(|w| w.contains("interface")),
            "expected warning about 'interface' field, got: {warnings:?}"
        );
    }

    #[test]
    fn synthesized_manifest_for_no_manifest_dir() {
        // Use a temp dir with no plugin.json.
        let tmpdir = tempfile::tempdir().unwrap();
        std::fs::write(tmpdir.path().join("hello.md"), b"# Hello").unwrap();
        // Rename the temp dir to have a known name by creating a sub-dir.
        let plugin_dir = tmpdir.path().join("my-test-plugin");
        std::fs::create_dir(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join("README.md"), b"content").unwrap();
        let fs = PluginFileSet::from_dir(&plugin_dir).unwrap();
        let (manifest, warnings) = fs.manifest().unwrap();
        assert_eq!(manifest.name, "my-test-plugin");
        assert!(warnings.iter().any(|w| w.contains("no plugin.json")));
    }

    #[test]
    fn oversized_file_rejected() {
        let tmpdir = tempfile::tempdir().unwrap();
        let big = vec![b'x'; MAX_PLUGIN_FILE_BYTES + 1];
        std::fs::write(tmpdir.path().join("big.txt"), &big).unwrap();
        let err = PluginFileSet::from_dir(tmpdir.path()).unwrap_err();
        assert!(err.contains("exceeding the"), "error was: {err}");
    }
}
