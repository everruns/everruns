use super::{
    AgentPackage, File, FileSource, Format, MAX_FILE_BYTES, MAX_FILES, MAX_PACKAGE_BYTES, Result,
    error,
};
use crate::{InitialFile, session_file::SessionFile};
use globset::Glob;
use std::{
    collections::BTreeMap,
    io::{Cursor, Read, Write},
    path::{Component, Path},
};

/// Dot-prefixed path components allowed by default in initial-files collection.
/// Covers the common dev-ecosystem assets shipped alongside agent packages.
/// Anything not in this list (and not in the user-declared
/// `initial_files_allow_hidden` opt-in) is skipped to prevent accidental upload
/// of secrets (`.env`, `.ssh/`, `.npmrc`, etc.).
pub const ALLOWED_DOT_ENTRIES: &[&str] = &[
    ".agents",
    ".github",
    ".vscode",
    ".claude",
    ".cursor",
    ".mcp.json",
    ".gitignore",
    ".gitattributes",
    ".editorconfig",
    ".prettierrc",
    ".prettierrc.json",
    ".prettierrc.yaml",
    ".prettierrc.yml",
    ".prettierrc.js",
    ".prettierrc.cjs",
    ".prettierrc.mjs",
    ".eslintrc",
    ".eslintrc.json",
    ".eslintrc.yaml",
    ".eslintrc.yml",
    ".eslintrc.js",
    ".eslintrc.cjs",
    ".eslintignore",
    ".nvmrc",
    ".node-version",
    ".python-version",
    ".tool-versions",
    ".dockerignore",
    ".rubocop.yml",
];

/// Hard-deny floor for hidden path components. Even if a user opts in via the
/// `initial_files_allow_hidden` manifest field, anything matching one of these
/// entries (exactly, by basename) is rejected. Protects against accidental
/// exfiltration of credentials, SSH/GPG keys, and shell history.
///
/// Keep this list strict and well-known. Adding speculative entries here is
/// safer than adding them to `ALLOWED_DOT_ENTRIES`.
pub const DENIED_DOT_ENTRIES: &[&str] = &[
    ".env",
    ".env.local",
    ".env.development",
    ".env.production",
    ".env.test",
    ".envrc",
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".gcloud",
    ".kube",
    ".docker",
    ".npmrc",
    ".yarnrc",
    ".pypirc",
    ".netrc",
    ".cargo",
    ".git",
    ".hg",
    ".svn",
    ".bash_history",
    ".zsh_history",
    ".python_history",
    ".node_repl_history",
];

pub(crate) fn safe_source(path: &str) -> bool {
    !path.is_empty()
        && !path.contains('\\')
        && !path.contains('\0')
        && !path.starts_with('/')
        && !path.contains(':')
        && !path.split('/').any(|part| part == "..")
}

pub(crate) fn workspace_path(path: &str) -> Result<String> {
    let path = path
        .strip_prefix("/workspace/")
        .unwrap_or(path)
        .trim_start_matches('/');
    if !safe_source(path) || path.split('/').any(|part| part.is_empty() || part == ".") {
        return Err(error("path", "invalid working-directory destination"));
    }
    Ok(format!("/{path}"))
}

pub(super) fn asset_path(path: &str) -> Result<String> {
    if !safe_source(path) || path.split('/').any(|s| s.is_empty() || s == ".") {
        return Err(error("source", "expected a relative package path"));
    }
    Ok(path.into())
}

pub(crate) fn allowed(path: &Path) -> bool {
    path.components().all(|component| {
        let Component::Normal(name) = component else {
            return false;
        };
        let name = name.to_string_lossy();
        !DENIED_DOT_ENTRIES.contains(&name.as_ref())
            && (!name.starts_with('.') || ALLOWED_DOT_ENTRIES.contains(&name.as_ref()))
    })
}

impl AgentPackage {
    // The canonical skill root is already the runtime path. Older skills/
    // folders remain importable; ambiguous automatic discovery requires intent.
    pub(super) fn skill_sources(&self, exists: impl Fn(&str) -> bool) -> Result<Vec<String>> {
        if !self.manifest.skills.is_empty() {
            return Ok(self.manifest.skills.clone());
        }
        if self.manifest.initial_files.iter().any(|file| match file {
            File::Inline(file) => file
                .path
                .trim_start_matches('/')
                .starts_with(".agents/skills/"),
            File::Source(file) => {
                file.source.starts_with(".agents/skills/") || file.source == ".agents/skills"
            }
            File::Pattern(source) => {
                source.starts_with(".agents/skills/") || source == ".agents/skills"
            }
        }) {
            return Ok(Vec::new());
        }
        let sources: Vec<_> = [".agents/skills", "skills"]
            .into_iter()
            .filter(|source| exists(source))
            .map(str::to_string)
            .collect();
        if sources.len() > 1 {
            return Err(error(
                "skills",
                "both .agents/skills and skills exist; declare skills explicitly",
            ));
        }
        Ok(sources)
    }

    /// Select declared assets from a virtual folder listing before reading bytes.
    /// Hosts use this to avoid uploading unrelated files beside an agent manifest.
    pub fn referenced_paths<'a>(
        &self,
        paths: impl IntoIterator<Item = &'a str>,
    ) -> Result<std::collections::BTreeSet<String>> {
        let paths: Vec<_> = paths.into_iter().collect();
        let mut sources: Vec<_> = self
            .manifest
            .initial_files
            .iter()
            .filter_map(|file| match file {
                File::Pattern(source) => Some(source.clone()),
                File::Source(file) => Some(file.source.clone()),
                File::Inline(_) => None,
            })
            .collect();
        if !self.files_declared && paths.iter().any(|path| path.starts_with("files/")) {
            sources.push("files".into());
        }
        sources.extend(self.skill_sources(|source| {
            paths
                .iter()
                .any(|path| path.starts_with(&format!("{source}/")))
        })?);
        let mut selected = std::collections::BTreeSet::new();
        if self.manifest.instructions.is_empty() {
            selected.insert(
                self.manifest
                    .instructions_file
                    .as_deref()
                    .unwrap_or("instructions.md")
                    .to_string(),
            );
        }
        for source in sources {
            let matcher = Glob::new(&source)
                .map_err(|e| error("files", e))?
                .compile_matcher();
            for path in &paths {
                if allowed(Path::new(path))
                    && (source == "."
                        || *path == source
                        || path.starts_with(&format!("{source}/"))
                        || matcher.is_match(path))
                {
                    selected.insert((*path).to_string());
                }
            }
        }
        Ok(selected)
    }

    pub(super) fn resolve_assets(
        &mut self,
        entries: BTreeMap<String, Vec<u8>>,
        defaults: bool,
    ) -> Result<()> {
        let files_declared = self.files_declared;
        let default_skills = self
            .skill_sources(|source| entries.keys().any(|p| p.starts_with(&format!("{source}/"))))?;
        let m = &mut self.manifest;
        if m.instructions.is_empty() {
            let source = m.instructions_file.as_deref().unwrap_or("instructions.md");
            m.instructions = String::from_utf8(
                entries
                    .get(source)
                    .ok_or_else(|| error("instructions_file", format!("missing {source}")))?
                    .clone(),
            )
            .map_err(|e| error("instructions_file", e))?;
        }
        m.instructions_file = None;
        let mut declared = std::mem::take(&mut m.initial_files);
        if !files_declared
            && declared.is_empty()
            && defaults
            && entries.keys().any(|s| s.starts_with("files/"))
        {
            declared.push(File::Source(FileSource {
                source: "files".into(),
                path: Some("/".into()),
                is_readonly: true,
            }));
        }
        let mut resolved = Vec::new();
        for file in declared {
            let (source, destination, readonly) = match file {
                File::Inline(file) => {
                    resolved.push(File::Inline(file));
                    continue;
                }
                File::Pattern(pattern) => (pattern, None, true),
                File::Source(source) => (source.source, source.path, source.is_readonly),
            };
            let matcher = Glob::new(&source)
                .map_err(|e| error("source", e))?
                .compile_matcher();
            let mut count = 0;
            for (path, bytes) in &entries {
                let directory = path.strip_prefix(&format!("{source}/"));
                if source != "."
                    && path != &source
                    && directory.is_none()
                    && !matcher.is_match(path)
                {
                    continue;
                }
                if !allowed(Path::new(path)) {
                    continue;
                }
                count += 1;
                let target = match &destination {
                    Some(target) if directory.is_some() => format!(
                        "{}/{}",
                        target.trim_end_matches('/'),
                        directory.unwrap_or_default()
                    ),
                    Some(target) => target.clone(),
                    None => format!("/{path}"),
                };
                let (content, encoding) = SessionFile::encode_content(bytes);
                resolved.push(File::Inline(InitialFile {
                    path: workspace_path(&target)?,
                    content,
                    encoding,
                    is_readonly: readonly,
                }));
            }
            if count == 0 {
                return Err(error("files", format!("source matched no files: {source}")));
            }
        }
        let explicit_skills = resolved
            .iter()
            .any(|file| matches!(file, File::Inline(f) if f.path.starts_with("/.agents/skills/")));
        let skills = if m.skills.is_empty() && defaults && !explicit_skills {
            default_skills
        } else {
            std::mem::take(&mut m.skills)
        };
        let mut has_skills = false;
        for source in skills {
            let mut source_has_skills = false;
            if !safe_source(&source) {
                return Err(error("skills", "must be relative to the package"));
            }
            let prefix = format!("{}/", source.trim_end_matches('/'));
            let mut skill_dirs = std::collections::BTreeSet::new();
            for path in entries.keys().filter_map(|path| path.strip_prefix(&prefix)) {
                if let Some(dir) = path.strip_suffix("/SKILL.md") {
                    if dir.contains('/') {
                        return Err(error("skills", "each skill must be a direct subfolder"));
                    }
                    skill_dirs.insert(dir.to_string());
                }
            }
            for path in entries.keys().filter_map(|path| path.strip_prefix(&prefix)) {
                if let Some((dir, _)) = path.split_once('/')
                    && !skill_dirs.contains(dir)
                {
                    return Err(error("skills", format!("{source}/{dir} requires SKILL.md")));
                }
            }
            for dir in skill_dirs {
                let prompt_path = format!("{prefix}{dir}/SKILL.md");
                let prompt =
                    std::str::from_utf8(&entries[&prompt_path]).map_err(|e| error("skills", e))?;
                let parsed = crate::skill::parse_skill_md(prompt)
                    .map_err(|e| error("skills", format!("invalid SKILL.md: {e:?}")))?;
                if parsed.name != dir {
                    return Err(error("skills", "SKILL.md name must match its directory"));
                }
                for (path, bytes) in &entries {
                    if !allowed(Path::new(path)) {
                        continue;
                    }
                    if let Some(relative) = path.strip_prefix(&format!("{prefix}{dir}/")) {
                        let (content, encoding) = SessionFile::encode_content(bytes);
                        resolved.push(File::Inline(InitialFile {
                            path: format!("/.agents/skills/{dir}/{relative}"),
                            content,
                            encoding,
                            is_readonly: true,
                        }));
                    }
                }
                has_skills = true;
                source_has_skills = true;
            }
            if !source_has_skills
                && (source != "skills" || entries.keys().any(|p| p.starts_with(&prefix)))
            {
                return Err(error("skills", format!("no skills found in {source}")));
            }
        }
        m.skills.clear();
        if has_skills && !m.capabilities.iter().any(|c| c.capability_id() == "skills") {
            m.capabilities
                .push(everruns_contracts::capability::CapabilityRef::new("skills"));
        }
        m.initial_files = resolved;
        super::defaults(m);
        self.validate()
    }

    // THREAT[TM-FS-020, TM-DOS-001]: validate archive paths and bound actual
    // decompressed bytes before interpreting any portable agent definition.
    pub fn from_zip(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_PACKAGE_BYTES {
            return Err(error("zip", "archive exceeds 10 MiB"));
        }
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| error("zip", e))?;
        if archive.len() > MAX_FILES + 20 {
            return Err(error("zip", "too many archive entries"));
        }
        let mut entries = BTreeMap::new();
        let mut total = 0;
        for i in 0..archive.len() {
            let mut file = archive.by_index(i).map_err(|e| error("zip", e))?;
            if file.is_dir() {
                continue;
            }
            let path = asset_path(file.name())?;
            if file
                .unix_mode()
                .is_some_and(|mode| mode & 0o170000 == 0o120000)
            {
                return Err(error("zip", "symlinks are not allowed"));
            }
            let limit = if path.starts_with("agent.") {
                MAX_PACKAGE_BYTES
            } else {
                MAX_FILE_BYTES
            };
            let mut content = Vec::new();
            (&mut file)
                .take((limit + 1) as u64)
                .read_to_end(&mut content)
                .map_err(|e| error("zip", e))?;
            total += content.len();
            if content.len() > limit || total > MAX_PACKAGE_BYTES {
                return Err(error("zip", "expanded archive exceeds limits"));
            }
            if entries.insert(path, content).is_some() {
                return Err(error("zip", "duplicate archive path"));
            }
        }
        Self::from_entries(entries)
    }

    /// Materialize a bounded package from a host-provided virtual directory.
    /// Entries are relative names and bytes, never worker-host filesystem paths.
    pub fn from_entries(entries: BTreeMap<String, Vec<u8>>) -> Result<Self> {
        if entries.len() > MAX_FILES + 20
            || entries.values().map(Vec::len).sum::<usize>() > MAX_PACKAGE_BYTES
        {
            return Err(error("assets", "package exceeds limits"));
        }
        for (path, bytes) in &entries {
            asset_path(path)?;
            let limit = if path.starts_with("agent.") {
                MAX_PACKAGE_BYTES
            } else {
                MAX_FILE_BYTES
            };
            if bytes.len() > limit {
                return Err(error(path, "file exceeds size limit"));
            }
        }
        let names = [
            "agent.toml",
            "agent.md",
            "agent.yaml",
            "agent.yml",
            "agent.json",
        ];
        let manifests: Vec<_> = entries
            .keys()
            .filter(|path| names.contains(&path.as_str()))
            .cloned()
            .collect();
        if manifests.len() != 1 {
            return Err(error(
                "zip",
                "archive root must contain exactly one agent manifest",
            ));
        }
        let manifest = &manifests[0];
        let text = std::str::from_utf8(&entries[manifest]).map_err(|e| error("manifest", e))?;
        let mut package = Self::parse(text, Format::from_extension(Path::new(manifest)))?;
        package.resolve_assets(entries, true)?;
        package.files()?;
        Ok(package)
    }

    pub fn folder_entries(&self) -> Result<BTreeMap<String, Vec<u8>>> {
        let files = self.files()?;
        let mut manifest = self.manifest.clone();
        let instructions = std::mem::take(&mut manifest.instructions);
        manifest.instructions_file = Some("instructions.md".into());
        manifest.initial_files.clear();
        // Every file has an explicit mapping. This also preserves writable
        // skill files authored directly as initial_files, without rediscovery.
        let mut entries = BTreeMap::new();
        entries.insert("instructions.md".into(), instructions.into_bytes());
        for file in files {
            let source = file.path.trim_start_matches('/').to_string();
            let source = asset_path(&source)?;
            // Explicit inline bytes may target hidden paths. Keep them inline
            // rather than exporting a source that host-file collection rejects.
            if !allowed(Path::new(&source))
                || matches!(
                    source.split('/').next().unwrap_or_default(),
                    "agent.toml"
                        | "agent.md"
                        | "agent.yaml"
                        | "agent.yml"
                        | "agent.json"
                        | "instructions.md"
                )
            {
                let mut file = file;
                file.path = source;
                manifest.initial_files.push(File::Inline(file));
                continue;
            }
            entries.insert(
                source.clone(),
                SessionFile::decode_content(&file.content, &file.encoding)
                    .map_err(|e| error("content", e))?,
            );
            manifest.initial_files.push(File::Source(FileSource {
                source,
                path: None,
                is_readonly: file.is_readonly,
            }));
        }
        // Explicit files already contain the complete skill trees.
        manifest.skills.clear();
        let text = toml::to_string_pretty(&manifest).map_err(|e| error("toml", e))?;
        entries.insert("agent.toml".into(), text.into_bytes());
        Ok(entries)
    }

    pub fn to_zip(&self) -> Result<Vec<u8>> {
        let entries = self.folder_entries()?;
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (path, bytes) in entries {
            writer
                .start_file(
                    path,
                    zip::write::SimpleFileOptions::default()
                        .compression_method(zip::CompressionMethod::Deflated),
                )
                .map_err(|e| error("zip", e))?;
            writer.write_all(&bytes).map_err(|e| error("zip", e))?;
        }
        Ok(writer.finish().map_err(|e| error("zip", e))?.into_inner())
    }
}
