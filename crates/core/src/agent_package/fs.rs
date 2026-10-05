//! Native disk effects; the portable package codec has no filesystem access.
use super::assets::{allowed, asset_path, safe_source};
use super::{
    AgentPackage, File, Format, MAX_FILE_BYTES, MAX_FILES, MAX_PACKAGE_BYTES, Result, error,
};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
};

fn collect(
    root: &Path,
    dir: &Path,
    entries: &mut BTreeMap<String, Vec<u8>>,
    total: &mut usize,
    visited: &mut usize,
    depth: usize,
    selection: (&str, &globset::GlobMatcher),
) -> Result<()> {
    let (source, matcher) = selection;
    // THREAT[TM-DOS-001]: empty directory trees must not bypass file-count limits.
    if depth > 32 {
        return Err(error("assets", "folder nesting exceeds 32 levels"));
    }
    for entry in fs::read_dir(dir).map_err(|e| error("source", e))? {
        *visited += 1;
        if *visited > 1024 {
            return Err(error("assets", "too many filesystem entries"));
        }
        let entry = entry.map_err(|e| error("source", e))?;
        let path = entry.path();
        let relative = path.strip_prefix(root).map_err(|e| error("source", e))?;
        if !allowed(relative) {
            continue;
        }
        let metadata = fs::symlink_metadata(&path).map_err(|e| error("source", e))?;
        if metadata.file_type().is_symlink() {
            return Err(error(
                relative.display().to_string(),
                "symlinks are not allowed",
            ));
        }
        if metadata.is_dir() {
            collect(root, &path, entries, total, visited, depth + 1, selection)?;
        } else if metadata.is_file() {
            let relative_name = relative.to_string_lossy();
            if source != "."
                && relative_name != source
                && !relative_name.starts_with(&format!("{source}/"))
                && !matcher.is_match(relative)
            {
                continue;
            }
            if entries.len() >= MAX_FILES + 2 {
                return Err(error("assets", "too many files"));
            }
            let bytes = read(&path)?;
            *total += bytes.len();
            if *total > MAX_PACKAGE_BYTES {
                return Err(error("assets", "expanded package exceeds 10 MiB"));
            }
            entries.insert(relative.to_string_lossy().replace('\\', "/"), bytes);
        }
    }
    Ok(())
}

fn read(path: &Path) -> Result<Vec<u8>> {
    read_bounded(path, MAX_FILE_BYTES)
}
fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path).map_err(|e| error(path.display().to_string(), e))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(error(
            path.display().to_string(),
            "expected a regular file, not a symlink",
        ));
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|e| error("source", e))?
        .take((limit + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| error("source", e))?;
    if bytes.len() > limit {
        return Err(error(path.display().to_string(), "file exceeds size limit"));
    }
    Ok(bytes)
}

impl AgentPackage {
    /// Load a manifest, a conventional agent folder, or a ZIP. All source reads
    /// are bounded and confined to the selected package, never the worker host.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(error("assets", "package root cannot be a symlink"));
        }
        if path.extension().is_some_and(|s| s == "zip") {
            let mut bytes = Vec::new();
            fs::File::open(path)
                .map_err(|e| error("zip", e))?
                .take((MAX_PACKAGE_BYTES + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(|e| error("zip", e))?;
            return Self::from_zip(&bytes);
        }
        let (root, manifest) = if path.is_dir() {
            let manifests: Vec<_> = [
                "agent.toml",
                "agent.md",
                "agent.yaml",
                "agent.yml",
                "agent.json",
            ]
            .iter()
            .map(|name| path.join(name))
            .filter(|p| p.is_file())
            .collect();
            if manifests.len() != 1 {
                return Err(error(
                    "manifest",
                    "folder must contain exactly one agent.toml, agent.md, agent.yaml or agent.json",
                ));
            }
            (path, manifests[0].clone())
        } else {
            (path.parent().unwrap_or(Path::new(".")), path.to_path_buf())
        };
        let text = String::from_utf8(read_bounded(&manifest, MAX_PACKAGE_BYTES)?)
            .map_err(|e| error("manifest", e))?;
        let mut package = Self::parse(&text, Format::from_extension(&manifest))?;
        let conventional = manifest
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| {
                matches!(
                    n,
                    "agent.toml" | "agent.md" | "agent.yaml" | "agent.yml" | "agent.json"
                )
            });
        let folder_defaults = path.is_dir()
            || (conventional
                && (root.join("files").is_dir()
                    || root.join("skills").is_dir()
                    || root.join(".agents/skills").is_dir()));
        let needs_assets = folder_defaults
            || package.manifest.instructions_file.is_some()
            || !package.manifest.skills.is_empty()
            || package
                .manifest
                .initial_files
                .iter()
                .any(|f| !matches!(f, File::Inline(_)))
            || (package.manifest.instructions.is_empty() && root.join("instructions.md").is_file());
        if needs_assets {
            // Read declared roots only. Never recursively upload an arbitrary
            // repository just because its agent.toml happens to live there.
            let mut entries = BTreeMap::new();
            let mut total = 0;
            let mut visited = 0;
            let prompt = package
                .manifest
                .instructions_file
                .as_deref()
                .unwrap_or("instructions.md");
            for ancestor in Path::new(asset_path(prompt)?.as_str())
                .ancestors()
                .filter(|p| !p.as_os_str().is_empty())
            {
                if fs::symlink_metadata(root.join(ancestor))
                    .is_ok_and(|m| m.file_type().is_symlink())
                {
                    return Err(error("instructions_file", "symlinks are not allowed"));
                }
            }
            if root.join(prompt).exists() {
                entries.insert(asset_path(prompt)?, read(&root.join(asset_path(prompt)?))?);
            }
            let mut sources: Vec<String> = package
                .manifest
                .initial_files
                .iter()
                .filter_map(|file| match file {
                    File::Pattern(p) => Some(p.clone()),
                    File::Source(s) => Some(s.source.clone()),
                    _ => None,
                })
                .collect();
            if !package.files_declared
                && sources.is_empty()
                && folder_defaults
                && root.join("files").exists()
            {
                sources.push("files".into());
            }
            let skills = package.skill_sources(|source| root.join(source).exists())?;
            sources.extend(skills);
            for source in sources {
                if !safe_source(&source) {
                    return Err(error("source", "must stay inside the package"));
                }
                let prefix = source
                    .split('/')
                    .take_while(|part| !part.contains(['*', '?', '[', '{']))
                    .collect::<Vec<_>>()
                    .join("/");
                let prefix = if prefix.is_empty() || prefix == "." {
                    "."
                } else {
                    &prefix
                };
                if !allowed(Path::new(prefix)) && prefix != "." {
                    return Err(error("source", "hidden credential paths are not allowed"));
                }
                for ancestor in Path::new(prefix)
                    .ancestors()
                    .filter(|p| !p.as_os_str().is_empty())
                {
                    if fs::symlink_metadata(root.join(ancestor))
                        .is_ok_and(|m| m.file_type().is_symlink())
                    {
                        return Err(error("source", "symlinks are not allowed"));
                    }
                }
                let selected = root.join(prefix);
                if !selected.exists() {
                    return Err(error("source", format!("missing package source {source}")));
                }
                let metadata = fs::symlink_metadata(&selected).map_err(|e| error("source", e))?;
                if metadata.file_type().is_symlink() {
                    return Err(error("source", "symlinks are not allowed"));
                }
                if metadata.is_dir() {
                    let matcher = globset::Glob::new(&source)
                        .map_err(|e| error("files", e))?
                        .compile_matcher();
                    collect(
                        root,
                        &selected,
                        &mut entries,
                        &mut total,
                        &mut visited,
                        0,
                        (&source, &matcher),
                    )?;
                } else {
                    entries.insert(asset_path(prefix)?, read(&selected)?);
                }
            }
            package.resolve_assets(entries, folder_defaults)?;
        }
        package.files()?;
        Ok(package)
    }

    /// Write a package without overwriting files or traversing symlinks.
    pub fn write_folder(&self, destination: impl AsRef<Path>) -> Result<()> {
        let destination = destination.as_ref();
        if destination.exists()
            && fs::read_dir(destination)
                .map_err(|e| error("destination", e))?
                .next()
                .is_some()
        {
            return Err(error("destination", "must be a new or empty directory"));
        }
        if fs::symlink_metadata(destination).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(error("destination", "symlinks are not allowed"));
        }
        let entries = self.folder_entries()?;
        fs::create_dir_all(destination).map_err(|e| error("destination", e))?;
        for (path, content) in entries {
            let target = destination.join(asset_path(&path)?);
            fs::create_dir_all(
                target
                    .parent()
                    .ok_or_else(|| error("destination", "asset requires a parent directory"))?,
            )
            .map_err(|e| error("destination", e))?;
            let mut file = OpenOptions::new()
                .create_new(true)
                .write(true)
                .open(target)
                .map_err(|e| error("destination", e))?;
            file.write_all(&content)
                .map_err(|e| error("destination", e))?;
        }
        Ok(())
    }
}
