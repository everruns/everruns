//! Code on this machine: the disk is read directly and language servers are
//! child processes.
//!
//! Edits proposed by a language server are applied one directory component at
//! a time, relative to an already-open directory and refusing symlinks, so a
//! concurrent rename of an ancestor cannot redirect a write outside the
//! workspace. That care matters here and not in a sandbox: this is a person's
//! own machine.

use super::{
    CodeHost, FileOp, ServerCommand, ServerIo, SourceTree, normalize_lexically,
    protected_component, resolve_scope,
};
use crate::source::SourceFilter;
use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use everruns_contracts::runtime::tool_context::ToolContext;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

type RootProvider = dyn Fn() -> Result<PathBuf> + Send + Sync;

/// The workspace is a directory on this machine.
#[derive(Clone)]
pub struct LocalCodeHost {
    root: Arc<RootProvider>,
}

impl LocalCodeHost {
    /// A workspace fixed at `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            root: Arc::new(move || Ok(root.clone())),
        }
    }

    /// A workspace whose root can move during a session (a host that switches
    /// to a git worktree). The provider is asked on every call.
    pub fn with_root_provider(
        provider: impl Fn() -> Result<PathBuf> + Send + Sync + 'static,
    ) -> Self {
        Self {
            root: Arc::new(provider),
        }
    }

    fn canonical_root(&self) -> Result<PathBuf> {
        (self.root)()?
            .canonicalize()
            .context("canonicalize workspace root")
    }
}

#[async_trait]
impl CodeHost for LocalCodeHost {
    fn scope_key(&self, _context: &ToolContext) -> String {
        "local".to_string()
    }

    fn display_root(&self) -> String {
        self.canonical_root()
            .map(|root| root.display().to_string())
            .unwrap_or_else(|_| ".".to_string())
    }

    async fn materialize(
        &self,
        _context: &ToolContext,
        path: Option<&str>,
        _filter: &SourceFilter,
    ) -> Result<SourceTree> {
        let root = self.canonical_root()?;
        let scope = match path {
            Some(path) => {
                let resolved = resolve_scope(&root, Some(path))?;
                let relative = resolved
                    .strip_prefix(&root)
                    .unwrap_or(&resolved)
                    .to_string_lossy()
                    .replace('\\', "/");
                (!relative.is_empty()).then_some(relative)
            }
            None => None,
        };
        Ok(SourceTree {
            display_root: root.display().to_string(),
            root,
            scope,
            note: None,
            _guard: None,
        })
    }

    async fn commit(
        &self,
        _context: &ToolContext,
        _tree: &SourceTree,
        _changed: &[String],
    ) -> Result<()> {
        // The tree is the workspace: the tool already wrote the files.
        Ok(())
    }

    async fn root(&self, _context: &ToolContext) -> Result<String> {
        Ok(self.canonical_root()?.to_string_lossy().into_owned())
    }

    async fn resolve_file(&self, _context: &ToolContext, path: &str) -> Result<String> {
        let root = self.canonical_root()?;
        let file = resolve_scope(&root, Some(path))?;
        if !file.is_file() {
            bail!("`{path}` is not a file in the workspace");
        }
        Ok(file.to_string_lossy().into_owned())
    }

    async fn project_root(&self, context: &ToolContext, _file: &str) -> Result<String> {
        self.root(context).await
    }

    async fn read_texts(
        &self,
        _context: &ToolContext,
        paths: &[String],
    ) -> Result<Vec<Option<String>>> {
        Ok(paths
            .iter()
            .map(|path| std::fs::read_to_string(path).ok())
            .collect())
    }

    async fn apply(&self, _context: &ToolContext, root: &str, ops: &[FileOp]) -> Result<()> {
        let root = Path::new(root);
        for op in ops {
            apply_one(root, op)?;
        }
        Ok(())
    }

    async fn launch(
        &self,
        _context: &ToolContext,
        _key: &str,
        command: &ServerCommand,
        root: &str,
    ) -> Result<ServerIo> {
        let mut child = tokio::process::Command::new(&command.command)
            .args(&command.args)
            .current_dir(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|err| {
                if err.kind() == std::io::ErrorKind::NotFound {
                    anyhow!(super::missing_server_message(&command.command))
                } else {
                    anyhow!("spawn language server `{}`: {err}", command.command)
                }
            })?;
        let stdout = child.stdout.take().context("capture server stdout")?;
        let stdin = child.stdin.take().context("capture server stdin")?;
        Ok(ServerIo {
            reader: Box::new(stdout),
            writer: Box::new(stdin),
            pump: None,
            guard: Some(Box::new(child)),
        })
    }
}

/// Re-check a target against the workspace: normalize `..` lexically (so a
/// path that does not exist yet cannot smuggle them past the prefix check),
/// then canonicalize the nearest existing ancestor so symlinks cannot escape.
fn checked_target(root: &Path, path: &str) -> Result<PathBuf> {
    let normalized = normalize_lexically(Path::new(path))?;
    let mut probe = normalized.clone();
    let mut suffix = Vec::new();
    let canonical = loop {
        match probe.canonicalize() {
            Ok(canonical) => break canonical,
            Err(_) => {
                let Some(name) = probe.file_name() else {
                    bail!("cannot resolve edit target {}", normalized.display());
                };
                suffix.push(name.to_owned());
                probe = probe
                    .parent()
                    .map(Path::to_path_buf)
                    .context("edit target has no parent")?;
            }
        }
    };
    let mut resolved = canonical;
    for part in suffix.into_iter().rev() {
        resolved.push(part);
    }
    if !resolved.starts_with(root) {
        bail!(
            "refusing a change outside the workspace: {}",
            normalized.display()
        );
    }
    if let Some(name) = protected_component(resolved.strip_prefix(root).unwrap_or(&resolved)) {
        bail!(
            "refusing a change inside the protected directory `{name}`: {}",
            normalized.display()
        );
    }
    Ok(resolved)
}

#[cfg(not(unix))]
fn apply_one(_root: &Path, _op: &FileOp) -> Result<()> {
    bail!("applying language-server edits is unsupported on this platform")
}

#[cfg(unix)]
fn apply_one(root: &Path, op: &FileOp) -> Result<()> {
    use rustix::fs::{AtFlags, Mode, OFlags};
    use std::os::unix::ffi::OsStrExt;

    match op {
        FileOp::Write { path, content } => {
            use std::io::{Seek, Write};
            let file = checked_target(root, path)?;
            let (parent, name) = open_parent(root, &file, false)?;
            let fd = rustix::fs::openat(
                &parent,
                name.as_bytes(),
                OFlags::RDWR | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .with_context(|| format!("open edit target {}", file.display()))?;
            let mut handle = std::fs::File::from(fd);
            handle
                .seek(std::io::SeekFrom::Start(0))
                .context("seek edit target")?;
            handle.set_len(0).context("truncate edit target")?;
            handle
                .write_all(content.as_bytes())
                .with_context(|| format!("write {}", file.display()))?;
        }
        FileOp::Create {
            path,
            overwrite,
            ignore_if_exists,
        } => {
            let file = checked_target(root, path)?;
            let (parent, name) = open_parent(root, &file, true)?;
            let mut flags = OFlags::WRONLY | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC;
            if *overwrite {
                flags |= OFlags::TRUNC;
            } else {
                flags |= OFlags::EXCL;
            }
            match rustix::fs::openat(&parent, name.as_bytes(), flags, Mode::from_raw_mode(0o666)) {
                Ok(_) => {}
                Err(err) if err == rustix::io::Errno::EXIST && *ignore_if_exists => {}
                Err(err) => {
                    return Err(err).with_context(|| format!("create file {}", file.display()));
                }
            }
        }
        FileOp::Rename { from, to } => {
            let old_file = checked_target(root, from)?;
            let new_file = checked_target(root, to)?;
            let (old_parent, old_name) = open_parent(root, &old_file, false)?;
            let (new_parent, new_name) = open_parent(root, &new_file, true)?;
            rustix::fs::renameat(
                &old_parent,
                old_name.as_bytes(),
                &new_parent,
                new_name.as_bytes(),
            )
            .context("rename file")?;
        }
        FileOp::Delete { path, recursive } => {
            let file = checked_target(root, path)?;
            let (parent, name) = open_parent(root, &file, false)?;
            let stat = rustix::fs::statat(&parent, name.as_bytes(), AtFlags::SYMLINK_NOFOLLOW)
                .context("inspect delete target")?;
            let file_type = rustix::fs::FileType::from_raw_mode(stat.st_mode);
            if file_type.is_symlink() {
                bail!("refusing to delete a symlink: {}", file.display());
            }
            if file_type.is_dir() {
                if *recursive {
                    bail!("recursive directory deletion from LSP edits is not supported safely");
                }
                rustix::fs::unlinkat(&parent, name.as_bytes(), AtFlags::REMOVEDIR)
                    .context("delete directory")?;
            } else {
                rustix::fs::unlinkat(&parent, name.as_bytes(), AtFlags::empty())
                    .context("delete file")?;
            }
        }
    }
    Ok(())
}

/// Open a target's parent one component at a time. Every lookup is relative to
/// an already-open directory and refuses symlinks.
#[cfg(unix)]
fn open_parent(
    root: &Path,
    file: &Path,
    create: bool,
) -> Result<(rustix::fd::OwnedFd, std::ffi::OsString)> {
    use rustix::fs::{Mode, OFlags};
    use std::os::unix::ffi::OsStrExt;

    let relative = file
        .strip_prefix(root)
        .context("workspace target is not beneath root")?;
    let name = relative
        .file_name()
        .context("workspace target has no file name")?
        .to_owned();
    let mut directory = rustix::fs::open(
        root,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .context("open workspace root")?;
    if let Some(parent) = relative.parent() {
        for component in parent.components() {
            let part = component.as_os_str();
            if create {
                match rustix::fs::mkdirat(&directory, part.as_bytes(), Mode::from_raw_mode(0o777)) {
                    Ok(()) => {}
                    Err(err) if err == rustix::io::Errno::EXIST => {}
                    Err(err) => return Err(err).context("create workspace-edit parent directory"),
                }
            }
            directory = rustix::fs::openat(
                &directory,
                part.as_bytes(),
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .context("open workspace-edit parent directory")?;
        }
    }
    Ok((directory, name))
}
