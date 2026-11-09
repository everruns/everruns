//! Code in the session's sandbox.
//!
//! The sandbox runs one command at a time and returns its output; there is no
//! open pipe and no shared disk. Everything here is built from three
//! operations the sandbox already offers: run a command, read a file, write a
//! file.
//!
//! - **Parsing** (ast-grep, repository maps) happens in this process. One
//!   command archives the matching source files, the archive is read back in
//!   parts, and it is unpacked into a private temporary directory. The archive
//!   is untrusted: only regular files with in-tree relative paths are kept.
//! - **Edits** go back as an archive unpacked in the sandbox, then the
//!   workspace is checkpointed once.
//! - **Language servers** run inside the sandbox, detached, with stdin on a
//!   FIFO and stdout appended to a file. A [`Relay`] moves bytes between that
//!   pair and the client whenever a tool call pumps it. Nothing but a shell and
//!   coreutils is required in the image.

use super::{
    CodeHost, FileOp, Pump, SKIP_DIRS, ServerCommand, ServerIo, SourceTree, missing_server_message,
    normalize_lexically, protected_component, workspace_relative,
};
use crate::failure::Passthrough;
use crate::source::SourceFilter;
use anyhow::{Context, Result, anyhow, bail};
use async_trait::async_trait;
use base64::Engine as _;
use everruns_contracts::runtime::sandbox_workspace::{SandboxWorkspace, SandboxWorkspaceExt};
use everruns_contracts::runtime::tool_context::ToolContext;
use everruns_contracts::session_sandbox::SessionSandboxExecResponse;
use std::io::Read as _;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::task::{Context as TaskContext, Poll};
use tokio::io::{AsyncWrite, AsyncWriteExt, DuplexStream};
use tokio::sync::{Mutex, mpsc};

const DISPLAY_ROOT: &str = "/workspace";
/// Files fetched for one parsing request.
const MAX_FETCH_FILES: usize = 20_000;
/// Compressed bytes fetched for one parsing request.
const MAX_FETCH_BYTES: u64 = 24 * 1024 * 1024;
/// Uncompressed bytes unpacked from one archive, whatever its header claims.
const MAX_UNPACKED_BYTES: u64 = 256 * 1024 * 1024;
/// One source file larger than this is not fetched (the tools skip it anyway).
const MAX_SOURCE_FILE_KIB: usize = 1024;
/// Providers cap one file read at 5 MiB; parts stay under it.
const PART_BYTES: &str = "3m";
/// A relay write up to this size rides inside the command text. A single
/// command-line argument is limited to 128 KiB on Linux.
const INLINE_WRITE_BYTES: usize = 48 * 1024;
/// Server output moved per pump tick.
const READ_CHUNK_BYTES: usize = 512 * 1024;
const COMMAND_TIMEOUT_MS: u64 = 120_000;

/// The session's sandbox as a [`CodeHost`]. Stateless: every call finds the
/// sandbox through the tool context, so the same value serves every session.
#[derive(Clone, Copy, Debug, Default)]
pub struct SandboxCodeHost;

fn sandbox(context: &ToolContext) -> Result<Arc<SandboxWorkspaceExt>> {
    context.extension::<SandboxWorkspaceExt>().context(
        "code navigation needs the session's sandbox, and this session has none; \
         give the agent a sandbox to use these tools",
    )
}

fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn scratch_dir() -> String {
    let unique = tempfile::Builder::new()
        .prefix("")
        .rand_bytes(16)
        .tempfile()
        .ok()
        .and_then(|file| {
            file.path()
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| format!("{:x}", std::process::id()));
    format!("/tmp/.everruns-code-{unique}")
}

async fn run(
    workspace: &dyn SandboxWorkspace,
    context: &ToolContext,
    script: &str,
    cwd: Option<&str>,
) -> Result<SessionSandboxExecResponse> {
    workspace
        .exec_raw(context, script, cwd, Some(COMMAND_TIMEOUT_MS))
        .await
        .map_err(Passthrough::wrap)
}

async fn read(
    workspace: &dyn SandboxWorkspace,
    context: &ToolContext,
    path: &str,
) -> Result<Vec<u8>> {
    workspace
        .read_file_raw(context, path)
        .await
        .map_err(Passthrough::wrap)
}

async fn write(
    workspace: &dyn SandboxWorkspace,
    context: &ToolContext,
    path: &str,
    content: &[u8],
) -> Result<()> {
    workspace
        .write_file_raw(context, path, content)
        .await
        .map_err(Passthrough::wrap)
}

/// Value of `KEY <number>` in a command's report lines.
fn report_number(stdout: &str, key: &str) -> Option<u64> {
    stdout.lines().find_map(|line| {
        line.trim()
            .strip_prefix(key)
            .and_then(|rest| rest.trim().parse().ok())
    })
}

/// Archive the files named by the list at `<scratch>/list` (one path per
/// line, relative to `base`), then read the archive back and return its
/// regular files as `(relative path, bytes)`.
async fn fetch_listed(
    workspace: &dyn SandboxWorkspace,
    context: &ToolContext,
    scratch: &str,
    base: &str,
    list_command: &str,
) -> Result<(Vec<(String, Vec<u8>)>, Option<String>)> {
    let dir = sh_quote(scratch);
    let script = format!(
        "mkdir -p {dir} || exit 1\n\
         {list_command}\n\
         n=$(wc -l < {dir}/list | tr -d ' ')\n\
         head -n {MAX_FETCH_FILES} {dir}/list > {dir}/kept\n\
         tar -czf {dir}/src.tgz -T {dir}/kept 2>/dev/null\n\
         b=$(wc -c < {dir}/src.tgz | tr -d ' ')\n\
         echo \"FILES $n\"; echo \"BYTES $b\"\n\
         if [ \"$b\" -le {MAX_FETCH_BYTES} ]; then (cd {dir} && split -b {PART_BYTES} src.tgz part.); fi\n\
         ls {dir} | grep '^part\\.' | sort | sed 's/^/PART /'\n"
    );
    let outcome = async {
        let response = run(workspace, context, &script, Some(base)).await?;
        let files = report_number(&response.stdout, "FILES").unwrap_or(0) as usize;
        let Some(bytes) = report_number(&response.stdout, "BYTES") else {
            let reason = response.stderr.trim();
            if reason.starts_with("path not found") {
                bail!("{reason}");
            }
            bail!("sandbox could not archive sources: {reason}");
        };
        if bytes > MAX_FETCH_BYTES {
            bail!(
                "the sources under this path are too large to analyze in one call \
                 ({} MiB compressed, limit {} MiB); pass a narrower `path`",
                bytes / (1024 * 1024),
                MAX_FETCH_BYTES / (1024 * 1024)
            );
        }
        let mut archive = Vec::with_capacity(bytes as usize);
        for line in response.stdout.lines() {
            let Some(part) = line.trim().strip_prefix("PART ") else {
                continue;
            };
            if !part.starts_with("part.") || part.contains('/') {
                continue;
            }
            archive.extend(read(workspace, context, &format!("{scratch}/{part}")).await?);
        }
        let note = (files > MAX_FETCH_FILES).then(|| {
            format!(
                "only the first {MAX_FETCH_FILES} of {files} source files were analyzed; \
                 pass a narrower `path` to cover the rest"
            )
        });
        Ok((unpack(&archive)?, note))
    }
    .await;
    // Best effort: the scratch directory is outside the workspace and the
    // sandbox is disposable, so a failed cleanup costs disk, not correctness.
    let _ = run(workspace, context, &format!("rm -rf {dir}"), None).await;
    outcome
}

/// Regular files of a gzip'd tar from the sandbox. The archive is untrusted
/// input: links, devices, absolute paths and `..` are dropped, and unpacking
/// stops at a fixed budget whatever the headers say.
fn unpack(archive: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    let mut files = Vec::new();
    if archive.is_empty() {
        return Ok(files);
    }
    let mut total = 0u64;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in tar.entries().context("read source archive")? {
        let mut entry = entry.context("read source archive entry")?;
        if entry.header().entry_type() != tar::EntryType::Regular {
            continue;
        }
        let path = entry.path().context("read archive entry path")?;
        let Some(relative) = safe_relative(&path) else {
            continue;
        };
        let mut content = Vec::new();
        (&mut entry)
            .take(MAX_UNPACKED_BYTES - total + 1)
            .read_to_end(&mut content)
            .context("read archive entry")?;
        total += content.len() as u64;
        if total > MAX_UNPACKED_BYTES {
            bail!("the source archive unpacks to more than the allowed size");
        }
        files.push((relative, content));
    }
    Ok(files)
}

/// `a/b/c` for an in-tree relative path; `None` for anything else.
fn safe_relative(path: &Path) -> Option<String> {
    use std::path::Component;
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(part) => parts.push(part.to_str()?.to_string()),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn pack(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    let mut builder = tar::Builder::new(encoder);
    for (path, content) in files {
        let mut header = tar::Header::new_gnu();
        header.set_size(content.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, path, content.as_slice())
            .with_context(|| format!("archive {path}"))?;
    }
    builder
        .into_inner()
        .context("finish archive")?
        .finish()
        .context("finish archive")
}

/// Write `files` (paths relative to `base`) into the sandbox. Existing files
/// keep their mode: the archive is unpacked without restoring permissions.
async fn upload(
    workspace: &dyn SandboxWorkspace,
    context: &ToolContext,
    base: &str,
    files: &[(String, Vec<u8>)],
) -> Result<()> {
    match files {
        [] => return Ok(()),
        [(path, content)] => {
            return write(
                workspace,
                context,
                &format!("{}/{path}", base.trim_end_matches('/')),
                content,
            )
            .await;
        }
        _ => {}
    }
    let scratch = scratch_dir();
    let dir = sh_quote(&scratch);
    let archive = format!("{scratch}/in.tgz");
    let made = run(workspace, context, &format!("mkdir -p {dir}"), None).await?;
    if made.exit_code != 0 {
        bail!(
            "sandbox could not create a scratch directory: {}",
            made.stderr.trim()
        );
    }
    let outcome = async {
        write(workspace, context, &archive, &pack(files)?).await?;
        let response = run(
            workspace,
            context,
            &format!(
                "tar -xzf {dir}/in.tgz --no-same-permissions --no-same-owner -C {} 2>&1 \
                 || tar -xzf {dir}/in.tgz -C {} 2>&1",
                sh_quote(base),
                sh_quote(base)
            ),
            None,
        )
        .await?;
        if response.exit_code != 0 {
            bail!("sandbox could not write files: {}", response.stdout.trim());
        }
        Ok(())
    }
    .await;
    let _ = run(workspace, context, &format!("rm -rf {dir}"), None).await;
    outcome
}

/// `find` expression selecting source files under `start`.
fn find_command(start: &str, filter: &SourceFilter) -> String {
    let prune = SKIP_DIRS
        .iter()
        .map(|name| format!("-name {}", sh_quote(name)))
        .collect::<Vec<_>>()
        .join(" -o ");
    let mut names: Vec<String> = filter
        .extensions
        .iter()
        .map(|extension| format!("-name {}", sh_quote(&format!("*.{extension}"))))
        .collect();
    names.extend(
        filter
            .filenames
            .iter()
            .map(|name| format!("-name {}", sh_quote(name))),
    );
    let names = if names.is_empty() {
        "-true".to_string()
    } else {
        names.join(" -o ")
    };
    format!(
        "find {} \\( -type d \\( {prune} \\) -prune \\) -o \
         \\( -type f \\( {names} \\) -size -{MAX_SOURCE_FILE_KIB}k -print \\)",
        sh_quote(start)
    )
}

/// Refuse a change target outside `root` or inside a protected directory.
fn checked_target<'a>(root: &str, path: &'a str) -> Result<&'a str> {
    let normalized = normalize_lexically(Path::new(path))?;
    let relative = normalized
        .strip_prefix(root)
        .map_err(|_| anyhow!("refusing a change outside the workspace: {path}"))?;
    if normalized.to_str() != Some(path) {
        bail!("refusing a change to a non-canonical path: {path}");
    }
    if let Some(name) = protected_component(relative) {
        bail!("refusing a change inside the protected directory `{name}`: {path}");
    }
    Ok(path)
}

#[async_trait]
impl CodeHost for SandboxCodeHost {
    fn scope_key(&self, context: &ToolContext) -> String {
        format!("sandbox:{}", context.session_id)
    }

    fn display_root(&self) -> String {
        DISPLAY_ROOT.to_string()
    }

    async fn materialize(
        &self,
        context: &ToolContext,
        path: Option<&str>,
        filter: &SourceFilter,
    ) -> Result<SourceTree> {
        let sandbox = sandbox(context)?;
        let workspace = sandbox.0.as_ref();
        let root = workspace
            .provider_root(context)
            .await
            .map_err(Passthrough::wrap)?;
        let scope = match path {
            Some(path) => workspace_relative(DISPLAY_ROOT, path)?,
            None => String::new(),
        };
        let start = if scope.is_empty() {
            ".".to_string()
        } else {
            format!("./{scope}")
        };
        let scratch = scratch_dir();
        // A file scope is fetched as is: the caller named it, so the tool
        // decides whether it is a supported source.
        let list_command = format!(
            "if [ -f {start} ]; then printf '%s\\n' {start} > {dir}/list; \
             elif [ -d {start} ]; then {find} > {dir}/list; \
             else echo \"path not found: \"{shown} >&2; exit 3; fi",
            start = sh_quote(&start),
            dir = sh_quote(&scratch),
            find = find_command(&start, filter),
            shown = sh_quote(path.unwrap_or(".").trim()),
        );
        let (files, note) =
            fetch_listed(workspace, context, &scratch, &root, &list_command).await?;

        let guard = tempfile::Builder::new()
            .prefix("everruns-code-")
            .tempdir()
            .context("create a private directory for fetched sources")?;
        let local_root = guard
            .path()
            .canonicalize()
            .context("canonicalize the fetched-sources directory")?;
        for (relative, content) in &files {
            let target = local_root.join(relative);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("create {}", parent.display()))?;
            }
            std::fs::write(&target, content)
                .with_context(|| format!("write {}", target.display()))?;
        }
        // A directory with no source files in it still exists.
        if !scope.is_empty() && !local_root.join(&scope).exists() {
            std::fs::create_dir_all(local_root.join(&scope))
                .context("create the fetched scope directory")?;
        }
        Ok(SourceTree {
            root: local_root,
            display_root: DISPLAY_ROOT.to_string(),
            scope: (!scope.is_empty()).then_some(scope),
            note,
            _guard: Some(guard),
        })
    }

    async fn commit(
        &self,
        context: &ToolContext,
        tree: &SourceTree,
        changed: &[String],
    ) -> Result<()> {
        if changed.is_empty() {
            return Ok(());
        }
        let sandbox = sandbox(context)?;
        let workspace = sandbox.0.as_ref();
        let root = workspace
            .provider_root(context)
            .await
            .map_err(Passthrough::wrap)?;
        let mut files = Vec::with_capacity(changed.len());
        for relative in changed {
            let Some(relative) = safe_relative(Path::new(relative)) else {
                bail!("refusing to write outside the workspace: {relative}");
            };
            let content = std::fs::read(tree.root.join(&relative))
                .with_context(|| format!("read rewritten file {relative}"))?;
            files.push((relative, content));
        }
        upload(workspace, context, &root, &files).await?;
        workspace
            .checkpoint(context)
            .await
            .map_err(Passthrough::wrap)
    }

    async fn root(&self, context: &ToolContext) -> Result<String> {
        sandbox(context)?
            .0
            .provider_root(context)
            .await
            .map_err(Passthrough::wrap)
    }

    async fn resolve_file(&self, context: &ToolContext, path: &str) -> Result<String> {
        let relative = workspace_relative(DISPLAY_ROOT, path)?;
        if relative.is_empty() {
            bail!("`{path}` is not a file in the workspace");
        }
        Ok(format!("{}/{relative}", self.root(context).await?))
    }

    async fn project_root(&self, context: &ToolContext, file: &str) -> Result<String> {
        let sandbox = sandbox(context)?;
        let workspace = sandbox.0.as_ref();
        let root = workspace
            .provider_root(context)
            .await
            .map_err(Passthrough::wrap)?;
        // Walk up from the file to the nearest directory holding `.git`,
        // never above the workspace root.
        let script = format!(
            "r={root}; d=$(dirname -- {file})\n\
             while [ \"$d\" != \"$r\" ] && [ \"$d\" != / ] && [ ! -e \"$d/.git\" ]; do d=$(dirname -- \"$d\"); done\n\
             case \"$d\" in \"$r\"|\"$r\"/*) printf '%s' \"$d\";; *) printf '%s' \"$r\";; esac",
            root = sh_quote(&root),
            file = sh_quote(file),
        );
        let response = run(workspace, context, &script, None).await?;
        let found = response.stdout.trim();
        Ok(if found.is_empty() || !found.starts_with(&root) {
            root
        } else {
            found.to_string()
        })
    }

    async fn read_texts(
        &self,
        context: &ToolContext,
        paths: &[String],
    ) -> Result<Vec<Option<String>>> {
        let sandbox = sandbox(context)?;
        let workspace = sandbox.0.as_ref();
        if paths.len() <= 3 {
            let mut texts = Vec::with_capacity(paths.len());
            for path in paths {
                texts.push(
                    workspace
                        .read_file_raw(context, path)
                        .await
                        .ok()
                        .and_then(|bytes| String::from_utf8(bytes).ok()),
                );
            }
            return Ok(texts);
        }
        // Many files: one archive instead of one round trip each.
        let scratch = scratch_dir();
        let list: String = paths
            .iter()
            .filter(|path| path.starts_with('/') && !path.contains('\n'))
            .map(|path| format!(".{path}\n"))
            .collect();
        let made = run(
            workspace,
            context,
            &format!("mkdir -p {}", sh_quote(&scratch)),
            None,
        )
        .await?;
        if made.exit_code != 0 {
            bail!(
                "sandbox could not create a scratch directory: {}",
                made.stderr.trim()
            );
        }
        write(
            workspace,
            context,
            &format!("{scratch}/wanted"),
            list.as_bytes(),
        )
        .await?;
        // Keep only the entries that are regular files, so a missing one
        // does not fail the archive.
        let list_command = format!(
            "while IFS= read -r f; do [ -f \"$f\" ] && printf '%s\\n' \"$f\"; done \
             < {dir}/wanted > {dir}/list",
            dir = sh_quote(&scratch)
        );
        let (files, _) = fetch_listed(workspace, context, &scratch, "/", &list_command).await?;
        let by_path: std::collections::HashMap<String, Vec<u8>> = files.into_iter().collect();
        Ok(paths
            .iter()
            .map(|path| {
                by_path
                    .get(path.trim_start_matches('/'))
                    .and_then(|bytes| String::from_utf8(bytes.clone()).ok())
            })
            .collect())
    }

    async fn apply(&self, context: &ToolContext, root: &str, ops: &[FileOp]) -> Result<()> {
        if ops.is_empty() {
            return Ok(());
        }
        let sandbox = sandbox(context)?;
        let workspace = sandbox.0.as_ref();
        let mut script = String::from("set -e\n");
        let mut writes = Vec::new();
        for op in ops {
            match op {
                FileOp::Write { path, content } => {
                    let path = checked_target(root, path)?;
                    writes.push((
                        path.trim_start_matches('/').to_string(),
                        content.clone().into_bytes(),
                    ));
                }
                FileOp::Create {
                    path,
                    overwrite,
                    ignore_if_exists,
                } => {
                    let path = sh_quote(checked_target(root, path)?);
                    script.push_str(&format!("mkdir -p \"$(dirname -- {path})\"\n"));
                    if *overwrite {
                        script.push_str(&format!(": > {path}\n"));
                    } else if *ignore_if_exists {
                        script.push_str(&format!("[ -e {path} ] || : > {path}\n"));
                    } else {
                        script.push_str(&format!(
                            "if [ -e {path} ]; then echo \"already exists: \"{path} >&2; exit 1; fi\n: > {path}\n"
                        ));
                    }
                }
                FileOp::Rename { from, to } => {
                    let from = sh_quote(checked_target(root, from)?);
                    let to = sh_quote(checked_target(root, to)?);
                    script.push_str(&format!(
                        "mkdir -p \"$(dirname -- {to})\"\nmv -- {from} {to}\n"
                    ));
                }
                FileOp::Delete { path, recursive } => {
                    let path = sh_quote(checked_target(root, path)?);
                    if *recursive {
                        bail!("recursive directory deletion from LSP edits is not supported");
                    }
                    script.push_str(&format!(
                        "if [ -L {path} ]; then echo \"refusing to delete a symlink: \"{path} >&2; exit 1; fi\n\
                         if [ -d {path} ]; then rmdir -- {path}; else rm -f -- {path}; fi\n"
                    ));
                }
            }
        }
        if script.lines().count() > 1 {
            let response = run(workspace, context, &script, None).await?;
            if response.exit_code != 0 {
                bail!(
                    "sandbox could not apply file operations: {}",
                    response.stderr.trim()
                );
            }
        }
        upload(workspace, context, "/", &writes).await?;
        workspace
            .checkpoint(context)
            .await
            .map_err(Passthrough::wrap)
    }

    async fn launch(
        &self,
        context: &ToolContext,
        key: &str,
        command: &ServerCommand,
        root: &str,
    ) -> Result<ServerIo> {
        let sandbox = sandbox(context)?;
        let workspace = sandbox.0.as_ref();
        let dir = format!("/tmp/.everruns-lsp/{}", relay_name(key));
        let quoted_dir = sh_quote(&dir);
        let argv = std::iter::once(command.command.as_str())
            .chain(command.args.iter().map(String::as_str))
            .map(sh_quote)
            .collect::<Vec<_>>()
            .join(" ");
        // A stale server for this key (an earlier process of ours that lost
        // its client) is replaced: a language server accepts one client.
        // The holder keeps the FIFO open for writing, so the server never
        // sees end-of-input between two of our writes.
        let script = format!(
            "d={quoted_dir}\n\
             for f in pid holder; do [ -f \"$d/$f\" ] && kill \"$(cat \"$d/$f\")\" 2>/dev/null; done\n\
             rm -rf \"$d\"; mkdir -p \"$d\" && mkfifo \"$d/in\" && : > \"$d/out\" || {{ echo SETUP_FAILED; exit 1; }}\n\
             command -v {program} >/dev/null 2>&1 || {{ rm -rf \"$d\"; echo MISSING; exit 0; }}\n\
             cd {root} || {{ echo SETUP_FAILED; exit 1; }}\n\
             if command -v setsid >/dev/null 2>&1; then s=setsid; else s=; fi\n\
             $s sh -c 'exec sleep 2147483647 > \"$0/in\"' \"$d\" </dev/null >/dev/null 2>&1 &\n\
             echo $! > \"$d/holder\"\n\
             $s sh -c 'd=\"$0\"; exec \"$@\" < \"$d/in\" >> \"$d/out\" 2>> \"$d/err\"' \"$d\" {argv} >/dev/null 2>&1 &\n\
             echo $! > \"$d/pid\"\n\
             echo STARTED\n",
            program = sh_quote(&command.command),
            root = sh_quote(root),
        );
        let response = run(workspace, context, &script, None).await?;
        if response.stdout.contains("MISSING") {
            bail!(missing_server_message(&command.command));
        }
        if !response.stdout.contains("STARTED") {
            bail!(
                "could not start language server `{}` in the sandbox: {}",
                command.command,
                response.stderr.trim()
            );
        }
        let (to_client, reader) = tokio::io::duplex(256 * 1024);
        let (sender, receiver) = mpsc::unbounded_channel();
        let relay = Arc::new(Relay {
            dir,
            offset: AtomicU64::new(0),
            outgoing: Mutex::new(receiver),
            sending: AtomicBool::new(false),
            to_client: Mutex::new(Some(to_client)),
        });
        Ok(ServerIo {
            reader: Box::new(reader),
            writer: Box::new(ChannelWriter(sender)),
            pump: Some(relay),
            guard: None,
        })
    }
}

/// A filesystem-safe name for a relay key.
fn relay_name(key: &str) -> String {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in key.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    let label: String = key
        .rsplit('|')
        .next()
        .unwrap_or("server")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(24)
        .collect();
    format!("{label}-{hash:016x}")
}

/// The client's write half: every write is queued for the next pump tick.
struct ChannelWriter(mpsc::UnboundedSender<Vec<u8>>);

impl AsyncWrite for ChannelWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut TaskContext<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Poll::Ready(
            self.0
                .send(buf.to_vec())
                .map(|()| buf.len())
                .map_err(|_| std::io::Error::from(std::io::ErrorKind::BrokenPipe)),
        )
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut TaskContext<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

/// Moves bytes between a client in this process and a language server in the
/// sandbox. See the module docs for the sandbox side.
struct Relay {
    dir: String,
    /// Bytes of the server's output already delivered to the client.
    offset: AtomicU64,
    outgoing: Mutex<mpsc::UnboundedReceiver<Vec<u8>>>,
    /// Set while client bytes are on their way to the server. Still set at
    /// the start of a tick means an earlier exchange was abandoned half-way
    /// (its tool call was cancelled): those bytes may or may not have arrived,
    /// so the stream cannot be trusted and the server is given up.
    sending: AtomicBool,
    /// Dropped when the server is gone, so the client reads end-of-input.
    to_client: Mutex<Option<DuplexStream>>,
}

impl Relay {
    async fn exchange(&self, context: &ToolContext, outgoing: Vec<u8>) -> Result<bool> {
        let sandbox = sandbox(context)?;
        let workspace = sandbox.0.as_ref();
        let dir = sh_quote(&self.dir);
        let mut script = format!("d={dir}\n");
        if !outgoing.is_empty() {
            if outgoing.len() <= INLINE_WRITE_BYTES {
                // Heredoc lines stay short: some shells cap line length.
                let encoded = base64::engine::general_purpose::STANDARD.encode(&outgoing);
                let wrapped = encoded
                    .as_bytes()
                    .chunks(76)
                    .map(|chunk| String::from_utf8_lossy(chunk).into_owned())
                    .collect::<Vec<_>>()
                    .join("\n");
                script.push_str(&format!(
                    "base64 -d >> \"$d/in\" <<'EVERRUNS_EOF'\n{wrapped}\nEVERRUNS_EOF\n"
                ));
            } else {
                let staged = format!("{}/staged", self.dir);
                write(workspace, context, &staged, &outgoing).await?;
                script.push_str("cat \"$d/staged\" >> \"$d/in\"; rm -f \"$d/staged\"\n");
            }
            // Give the server a moment, so the answer to a quick request
            // rides back on this same command.
            script.push_str("sleep 0.2 2>/dev/null || true\n");
        }
        let offset = self.offset.load(Ordering::SeqCst);
        script.push_str(&format!(
            "tail -c +{} \"$d/out\" 2>/dev/null | head -c {READ_CHUNK_BYTES} | base64 | tr -d '\\n'\n\
             echo\n\
             if kill -0 \"$(cat \"$d/pid\" 2>/dev/null)\" 2>/dev/null; then echo ALIVE; else echo GONE; fi\n",
            offset + 1
        ));
        if !outgoing.is_empty() {
            self.sending.store(true, Ordering::SeqCst);
        }
        let response = run(workspace, context, &script, None).await?;
        self.sending.store(false, Ordering::SeqCst);
        let mut lines = response.stdout.lines();
        let encoded = lines.next().unwrap_or("").trim();
        let alive = response.stdout.lines().any(|line| line.trim() == "ALIVE");
        let bytes = if encoded.is_empty() {
            Vec::new()
        } else {
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .context("decode language server output")?
        };
        if !bytes.is_empty() {
            self.offset.fetch_add(bytes.len() as u64, Ordering::SeqCst);
            let mut to_client = self.to_client.lock().await;
            if let Some(stream) = to_client.as_mut() {
                stream
                    .write_all(&bytes)
                    .await
                    .context("deliver language server output")?;
            }
        }
        if !alive && bytes.is_empty() {
            bail!("the language server in the sandbox is no longer running");
        }
        Ok(!bytes.is_empty() || !outgoing.is_empty())
    }
}

#[async_trait]
impl Pump for Relay {
    async fn tick(&self, context: &ToolContext) -> Result<bool> {
        // One exchange at a time: parallel tool calls share the relay.
        let Ok(mut receiver) = self.outgoing.try_lock() else {
            return Ok(false);
        };
        let moved = if self.sending.load(Ordering::SeqCst) {
            Err(anyhow!(
                "an exchange with the language server was interrupted"
            ))
        } else {
            let mut outgoing = Vec::new();
            while let Ok(chunk) = receiver.try_recv() {
                outgoing.extend(chunk);
            }
            self.exchange(context, outgoing).await
        };
        if moved.is_err() {
            // End-of-input for the client: it marks the server as exited.
            self.to_client.lock().await.take();
        }
        moved
    }
}

// The whole host against a real shell: a fake sandbox that runs each command
// with `sh` in a temporary directory.
#[cfg(all(test, unix))]
#[path = "sandbox_tests.rs"]
mod shell_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpack_keeps_only_in_tree_regular_files() {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::default(),
        ));
        let mut add = |path: &str, kind: tar::EntryType, content: &[u8]| {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(kind);
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            // `set_path` refuses `..`; write the raw name the way a hostile
            // archive would.
            let name = &mut header.as_old_mut().name;
            name[..path.len()].copy_from_slice(path.as_bytes());
            header.set_cksum();
            builder.append(&header, content).unwrap();
        };
        add("./src/lib.rs", tar::EntryType::Regular, b"fn a() {}");
        add("../escape.rs", tar::EntryType::Regular, b"x");
        add("/etc/passwd", tar::EntryType::Regular, b"x");
        add("src/link.rs", tar::EntryType::Symlink, b"");
        let archive = builder.into_inner().unwrap().finish().unwrap();

        let files = unpack(&archive).unwrap();
        assert_eq!(
            files,
            vec![("src/lib.rs".to_string(), b"fn a() {}".to_vec())]
        );
    }

    #[test]
    fn pack_roundtrips_through_unpack() {
        let files = vec![
            ("a/one.rs".to_string(), b"one".to_vec()),
            ("two.rs".to_string(), b"two".to_vec()),
        ];
        assert_eq!(unpack(&pack(&files).unwrap()).unwrap(), files);
    }

    #[test]
    fn checked_target_refuses_escapes_and_protected_directories() {
        assert!(checked_target("/home/ws", "/home/ws/app/src/lib.rs").is_ok());
        assert!(checked_target("/home/ws", "/home/other/lib.rs").is_err());
        assert!(checked_target("/home/ws", "/home/ws/../other/lib.rs").is_err());
        assert!(checked_target("/home/ws", "/home/ws/app/.git/config").is_err());
        assert!(checked_target("/home/ws", "/home/ws/app/node_modules/x.js").is_err());
    }

    #[test]
    fn find_command_prunes_and_filters() {
        let filter = SourceFilter {
            extensions: vec!["rs".to_string()],
            filenames: vec!["Dockerfile".to_string()],
        };
        let command = find_command("./app", &filter);
        assert!(command.starts_with("find './app'"), "{command}");
        assert!(command.contains("-name 'node_modules'"), "{command}");
        assert!(
            command.contains("-name '*.rs' -o -name 'Dockerfile'"),
            "{command}"
        );
    }

    #[test]
    fn relay_names_are_stable_and_filesystem_safe() {
        let name = relay_name("sandbox:abc|/home/ws/app|rust");
        assert!(name.starts_with("rust-"), "{name}");
        assert_eq!(name, relay_name("sandbox:abc|/home/ws/app|rust"));
        assert_ne!(name, relay_name("sandbox:abc|/home/ws/lib|rust"));
        assert!(
            name.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        );
    }
}
