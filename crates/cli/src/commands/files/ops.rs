// One-shot session filesystem operations backed directly by the SDK's
// `session_files()` client.
//
// These commands (cat/write/rm/mkdir/mv/cp/grep/stat) are thin wrappers over
// the typed SDK surface. The bidirectional `sync`/`push`/`pull` paths still use
// `RemoteClient` (raw reqwest) because they depend on the server's per-entry
// `content_hash` for change detection, which the SDK's `FileInfo`/`SessionFile`
// models do not expose. See `remote.rs` for that constraint.
//
// Design Decision: Binary detection mirrors the server (null bytes in first 8KB)
// and the existing RemoteClient — text files are sent inline, binary as base64.

use crate::output::{OutputFormat, print_field};
use anyhow::{Context, Result};
use base64::Engine;
use everruns_sdk::{CreateFileRequest, Everruns, UpdateFileRequest};

/// Read a file and print its content to stdout (or write to `--output`).
pub async fn cat(
    client: &Everruns,
    session_id: &str,
    path: &str,
    out_file: Option<&str>,
) -> Result<()> {
    let file = client
        .session_files()
        .read(session_id, path)
        .await
        .with_context(|| format!("Failed to read {}", path))?;

    let content = file.content.unwrap_or_default();
    let bytes = if file.encoding.as_deref() == Some("base64") {
        base64::engine::general_purpose::STANDARD
            .decode(content.as_bytes())
            .context("Failed to decode base64 content")?
    } else {
        content.into_bytes()
    };

    if let Some(path) = out_file {
        std::fs::write(path, &bytes).with_context(|| format!("Failed to write {}", path))?;
    } else {
        use std::io::Write;
        std::io::stdout().write_all(&bytes)?;
    }
    Ok(())
}

/// Create or update a file from a local file or stdin.
pub async fn write(
    client: &Everruns,
    output: OutputFormat,
    quiet: bool,
    session_id: &str,
    path: &str,
    from: Option<&str>,
    readonly: bool,
) -> Result<()> {
    let bytes = match from {
        Some(p) => std::fs::read(p).with_context(|| format!("Failed to read {}", p))?,
        None => {
            use std::io::Read;
            let mut buf = Vec::new();
            std::io::stdin()
                .read_to_end(&mut buf)
                .context("Failed to read stdin")?;
            buf
        }
    };

    // Detect binary: null bytes in first 8KB (mirrors server + RemoteClient).
    let is_binary = bytes.iter().take(8192).any(|&b| b == 0);
    let (encoded, encoding) = if is_binary {
        (
            base64::engine::general_purpose::STANDARD.encode(&bytes),
            "base64",
        )
    } else {
        (String::from_utf8_lossy(&bytes).into_owned(), "text")
    };

    let files = client.session_files();
    // Try create; if the file already exists, fall back to update.
    let mut create_req = CreateFileRequest::file(encoded.clone()).encoding(encoding);
    if readonly {
        create_req = create_req.is_readonly(true);
    }
    let result = files
        .create_with_options(session_id, path, create_req)
        .await;
    let file = match result {
        Ok(file) => file,
        Err(everruns_sdk::Error::Api { status: 409, .. }) => {
            let mut update_req = UpdateFileRequest::content(encoded).encoding(encoding);
            if readonly {
                update_req = update_req.is_readonly(true);
            }
            files
                .update_with_options(session_id, path, update_req)
                .await
                .with_context(|| format!("Failed to update {}", path))?
        }
        Err(e) => return Err(e).with_context(|| format!("Failed to write {}", path)),
    };

    if output.is_text() {
        if !quiet {
            println!("Wrote {} ({} bytes)", file.path, file.size_bytes);
        }
    } else {
        output.print_value(&file);
    }
    Ok(())
}

/// Remove a file or directory.
pub async fn rm(
    client: &Everruns,
    output: OutputFormat,
    quiet: bool,
    session_id: &str,
    path: &str,
    recursive: bool,
) -> Result<()> {
    let resp = client
        .session_files()
        .delete(session_id, path, Some(recursive))
        .await
        .with_context(|| format!("Failed to delete {}", path))?;

    if output.is_text() {
        if !quiet {
            println!("Deleted: {} (deleted={})", path, resp.deleted);
        }
    } else {
        output.print_value(&resp);
    }
    Ok(())
}

/// Create a directory.
pub async fn mkdir(
    client: &Everruns,
    output: OutputFormat,
    quiet: bool,
    session_id: &str,
    path: &str,
) -> Result<()> {
    let dir = client
        .session_files()
        .create_dir(session_id, path)
        .await
        .with_context(|| format!("Failed to create directory {}", path))?;

    if output.is_text() {
        if !quiet {
            println!("Created directory: {}", dir.path);
        }
    } else {
        output.print_value(&dir);
    }
    Ok(())
}

/// Move (rename) a file.
pub async fn mv(
    client: &Everruns,
    output: OutputFormat,
    quiet: bool,
    session_id: &str,
    src: &str,
    dest: &str,
) -> Result<()> {
    let file = client
        .session_files()
        .move_file(session_id, src, dest)
        .await
        .with_context(|| format!("Failed to move {} to {}", src, dest))?;

    if output.is_text() {
        if !quiet {
            println!("Moved {} -> {}", src, file.path);
        }
    } else {
        output.print_value(&file);
    }
    Ok(())
}

/// Copy a file.
pub async fn cp(
    client: &Everruns,
    output: OutputFormat,
    quiet: bool,
    session_id: &str,
    src: &str,
    dest: &str,
) -> Result<()> {
    let file = client
        .session_files()
        .copy_file(session_id, src, dest)
        .await
        .with_context(|| format!("Failed to copy {} to {}", src, dest))?;

    if output.is_text() {
        if !quiet {
            println!("Copied {} -> {}", src, file.path);
        }
    } else {
        output.print_value(&file);
    }
    Ok(())
}

/// Search file contents with a pattern.
pub async fn grep(
    client: &Everruns,
    output: OutputFormat,
    session_id: &str,
    pattern: &str,
    path_pattern: Option<&str>,
) -> Result<()> {
    let response = client
        .session_files()
        .grep(session_id, pattern, path_pattern)
        .await
        .context("Failed to grep session files")?;

    if output.is_text() {
        let mut any = false;
        for result in &response.data {
            for m in &result.matches {
                any = true;
                println!("{}:{}:{}", result.path, m.line_number, m.line);
            }
        }
        if !any {
            println!("No matches");
        }
    } else {
        output.print_value(&serde_json::json!({ "data": response.data }));
    }
    Ok(())
}

/// Show metadata for a file or directory.
pub async fn stat(
    client: &Everruns,
    output: OutputFormat,
    session_id: &str,
    path: &str,
) -> Result<()> {
    let stat = client
        .session_files()
        .stat(session_id, path)
        .await
        .with_context(|| format!("Failed to stat {}", path))?;

    if output.is_text() {
        print_field("Path", &stat.path);
        print_field("Name", &stat.name);
        print_field(
            "Type",
            if stat.is_directory {
                "directory"
            } else {
                "file"
            },
        );
        print_field("Size", &stat.size_bytes.to_string());
        print_field("Read-only", &stat.is_readonly.to_string());
        print_field("Created", &stat.created_at);
        print_field("Updated", &stat.updated_at);
    } else {
        output.print_value(&stat);
    }
    Ok(())
}
