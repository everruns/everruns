# FetchKit File Download

Extends fetchkit with a `FileSaver` trait abstraction so fetched content can be saved to files. Consumers provide the implementation — CLI uses real filesystem, everruns uses `SessionFileStore`.

## Design

### Core abstraction: `FileSaver` trait (in fetchkit)

```rust
/// Destination for saving fetched content to files.
///
/// Consumers implement this trait to control where bytes land:
/// - CLI: writes to real filesystem (`LocalFileSaver`)
/// - Everruns: writes to session virtual filesystem
/// - Tests: in-memory buffer
#[async_trait]
pub trait FileSaver: Send + Sync {
    /// Save raw bytes to the given path.
    /// Returns the canonical path where the file was written and bytes written.
    async fn save(&self, path: &str, bytes: &[u8]) -> Result<SaveResult, FileSaveError>;

    /// Check if a path is writable / allowed before fetching.
    /// Default: always allowed.
    async fn validate_path(&self, path: &str) -> Result<(), FileSaveError> {
        let _ = path;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct SaveResult {
    /// Canonical/normalized path where file was saved
    pub path: String,
    /// Bytes written
    pub bytes_written: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum FileSaveError {
    #[error("Path not allowed: {0}")]
    PathNotAllowed(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Save error: {0}")]
    Other(String),
}
```

### `LocalFileSaver` (built-in, for CLI)

```rust
/// Saves to real filesystem. Ships with fetchkit.
pub struct LocalFileSaver {
    /// Optional base directory. Paths resolved relative to this.
    /// If None, paths must be absolute.
    base_dir: Option<PathBuf>,
}
```

- Resolves relative paths against `base_dir`
- Creates parent directories automatically
- `validate_path` rejects path traversal (`..`) outside base_dir

### Extended abstraction: `FilesSaver` (unarchive support)

```rust
/// Extended saver that can unpack archives after download.
///
/// Builds on `FileSaver` — if the fetched content is an archive
/// (zip, tar.gz, tar), extract its contents into a directory.
#[async_trait]
pub trait FilesSaver: FileSaver {
    /// Save bytes and if the content is a recognized archive, extract it.
    /// Returns list of all files created.
    async fn save_and_extract(
        &self,
        path: &str,
        bytes: &[u8],
        content_type: Option<&str>,
    ) -> Result<ExtractResult, FileSaveError>;
}

#[derive(Debug, Clone)]
pub struct ExtractResult {
    /// Directory where files were extracted
    pub directory: String,
    /// Individual files created
    pub files: Vec<SaveResult>,
    /// Total bytes written
    pub total_bytes: u64,
}
```

- Separate trait so consumers can opt-in
- Archive detection by content-type + magic bytes
- Supported formats: zip, tar.gz, tar (extensible)

### FetchRequest changes

```rust
pub struct FetchRequest {
    // ... existing fields ...

    /// Save response body to this path instead of returning content inline.
    /// Requires a `FileSaver` to be provided at execution time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub save_to_file: Option<String>,

    /// If true and content is an archive, extract after download.
    /// Requires a `FilesSaver` implementation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract: Option<bool>,
}
```

### FetchResponse changes

```rust
pub struct FetchResponse {
    // ... existing fields ...

    /// Path where file was saved (when save_to_file was used)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_path: Option<String>,

    /// Bytes written to file
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes_written: Option<u64>,

    /// Extracted files (when extract was used)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extracted_files: Option<Vec<String>>,
}
```

### Execution flow

#### `Tool::execute_with_saver` (new method on fetchkit `Tool`)

```rust
impl Tool {
    /// Execute fetch with optional file saving.
    pub async fn execute_with_saver(
        &self,
        req: FetchRequest,
        saver: Option<&dyn FileSaver>,
    ) -> Result<FetchResponse, FetchError> { ... }

    /// Execute fetch with archive extraction support.
    pub async fn execute_with_files_saver(
        &self,
        req: FetchRequest,
        saver: Option<&dyn FilesSaver>,
    ) -> Result<FetchResponse, FetchError> { ... }
}
```

Flow when `save_to_file` is set:

1. `validate_path(path)` — fail fast before HTTP request
2. HTTP fetch — **skip binary content rejection** (binary is expected for downloads)
3. Stream response bytes (reuse `read_body_with_timeout`)
4. `saver.save(path, &bytes)` — write through trait
5. Return metadata response (no inline `content`, add `saved_path` + `bytes_written`)

Flow when `extract: true`:

1. Same as above but use `FilesSaver::save_and_extract`
2. Response includes `extracted_files` list

### `DefaultFetcher` changes

The key change in `DefaultFetcher::fetch`: when `save_to_file` is set, **skip the binary content rejection** and the HTML conversion paths. The fetcher needs access to the saver, so the `Fetcher` trait gets an optional saver parameter:

```rust
#[async_trait]
pub trait Fetcher: Send + Sync {
    // ... existing methods ...

    /// Fetch with file saving support.
    /// Default implementation delegates to `fetch()`.
    async fn fetch_to_file(
        &self,
        request: &FetchRequest,
        options: &FetchOptions,
        saver: &dyn FileSaver,
    ) -> Result<FetchResponse, FetchError> {
        // Default: fetch normally, then save content
        let response = self.fetch(request, options).await?;
        if let (Some(path), Some(content)) = (&request.save_to_file, &response.content) {
            saver.save(path, content.as_bytes()).await
                .map_err(|e| FetchError::FetcherError(e.to_string()))?;
        }
        Ok(response)
    }
}
```

`DefaultFetcher` overrides this with the optimized binary-aware path.

### `FetchOptions` changes

```rust
pub struct FetchOptions {
    // ... existing fields ...

    /// Enable save_to_file parameter in requests
    pub enable_save_to_file: bool,

    /// Enable extract parameter in requests
    pub enable_extract: bool,
}
```

Schema gating in `Tool::input_schema()` follows existing pattern (remove `save_to_file`/`extract` properties when disabled).

### `ToolBuilder` changes

```rust
impl ToolBuilder {
    /// Enable file download (save_to_file parameter)
    pub fn enable_save_to_file(mut self, enable: bool) -> Self { ... }

    /// Enable archive extraction (extract parameter)
    pub fn enable_extract(mut self, enable: bool) -> Self { ... }
}
```

### `FetchError` additions

```rust
pub enum FetchError {
    // ... existing variants ...

    /// File save failed
    #[error("Failed to save file: {0}")]
    SaveError(String),

    /// No FileSaver provided but save_to_file was requested
    #[error("File saving not available")]
    SaverNotAvailable,
}
```

## Everruns integration

In `crates/core/src/capabilities/web_fetch.rs`:

1. `WebFetchTool` switches to `execute_with_context` (like `WriteFileTool`)
2. When `save_to_file` is in arguments, construct a `SessionFileSaver` adapter:

```rust
/// Adapter: SessionFileStore -> fetchkit::FileSaver
struct SessionFileSaver {
    file_store: Arc<dyn SessionFileStore>,
    session_id: SessionId,
}

#[async_trait]
impl fetchkit::FileSaver for SessionFileSaver {
    async fn save(&self, path: &str, bytes: &[u8]) -> Result<SaveResult, FileSaveError> {
        // Encode binary as base64 for SessionFileStore
        let encoding = if is_likely_text(bytes) { "text" } else { "base64" };
        let content = if encoding == "base64" {
            base64::encode(bytes)
        } else {
            String::from_utf8_lossy(bytes).to_string()
        };

        let file = self.file_store
            .write_file(self.session_id.clone(), path, &content, encoding)
            .await
            .map_err(|e| FileSaveError::Other(e.to_string()))?;

        Ok(SaveResult {
            path: file.path,
            bytes_written: bytes.len() as u64,
        })
    }
}
```

3. Pass `Some(&saver)` to `fetchkit::Tool::execute_with_saver`
4. When `save_to_file` is absent, pass `None` — existing behavior unchanged

## API surface summary

### New public types in fetchkit

| Type | Kind | Purpose |
|------|------|---------|
| `FileSaver` | trait | Core abstraction for file output |
| `FilesSaver` | trait (extends FileSaver) | Archive extraction support |
| `LocalFileSaver` | struct | Built-in real-filesystem implementation |
| `SaveResult` | struct | Result of a save operation |
| `ExtractResult` | struct | Result of save+extract |
| `FileSaveError` | enum | File saving errors |

### New/changed methods

| Method | Change |
|--------|--------|
| `Tool::execute_with_saver()` | New — fetch + optional save |
| `Tool::execute_with_files_saver()` | New — fetch + optional extract |
| `Tool::execute()` | Unchanged — backward compatible |
| `ToolBuilder::enable_save_to_file()` | New — schema gating |
| `ToolBuilder::enable_extract()` | New — schema gating |
| `Fetcher::fetch_to_file()` | New default method |

### Backward compatibility

- `Tool::execute()` unchanged — no `FileSaver` needed
- `FetchRequest` new fields are `Option` with `skip_serializing_if`
- `FetchResponse` new fields are `Option` with `skip_serializing_if`
- Schema gating: `save_to_file`/`extract` hidden unless explicitly enabled
- Existing consumers see zero changes unless they opt in

## Implementation order

1. Add `FileSaver` trait + `LocalFileSaver` + `SaveResult` + `FileSaveError` to fetchkit
2. Add `save_to_file` to `FetchRequest`, `saved_path`/`bytes_written` to `FetchResponse`
3. Add `enable_save_to_file` to `FetchOptions`/`ToolBuilder`, schema gating
4. Implement `execute_with_saver` on `Tool`, `fetch_to_file` on `DefaultFetcher`
5. Add `FetchError::SaveError` / `FetchError::SaverNotAvailable`
6. Update `TOOL_LLMTXT` / `TOOL_DESCRIPTION`
7. Tests: unit tests for `LocalFileSaver`, integration test with wiremock
8. Bump fetchkit version
9. Everruns: `SessionFileSaver` adapter, `WebFetchTool` context-aware execution
10. (Future) `FilesSaver` trait + archive extraction
