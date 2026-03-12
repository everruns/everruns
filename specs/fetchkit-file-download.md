# FetchKit File Download

Extends fetchkit with a `FileSaver` trait abstraction so fetched content can be saved to files. Consumers provide the implementation — CLI uses real filesystem, everruns uses `SessionFileStore`.

## Status

**fetchkit 0.1.3 shipped** with the core `FileSaver` abstraction. This spec documents what shipped and what remains for the everruns integration.

## fetchkit 0.1.3 — shipped

See `fetchkit::file_saver` module for full source.

### `FileSaver` trait

```rust
#[async_trait]
pub trait FileSaver: Send + Sync {
    async fn save(&self, path: &str, bytes: &[u8]) -> Result<SaveResult, FileSaveError>;
    async fn validate_path(&self, path: &str) -> Result<(), FileSaveError> { Ok(()) }
}
```

### `LocalFileSaver` (built-in, for CLI)

- Resolves relative paths against optional `base_dir`
- Creates parent directories automatically
- `validate_path` rejects path traversal (`..`) outside base_dir
- Without `base_dir`, only absolute paths accepted

### `FetchRequest` / `FetchResponse` changes

- `FetchRequest::save_to_file: Option<String>` — destination path
- `FetchResponse::saved_path: Option<String>` — where file was written
- `FetchResponse::bytes_written: Option<u64>` — bytes saved

### `Tool::execute_with_saver`

```rust
pub async fn execute_with_saver(
    &self,
    req: FetchRequest,
    saver: Option<&dyn FileSaver>,
) -> Result<FetchResponse, FetchError>
```

- When `save_to_file` is `None` → delegates to `execute()` (backward compat)
- When `save_to_file` is set → validates path, fetches via `FetcherRegistry::fetch_to_file`, returns metadata-only response

### `Fetcher::fetch_to_file` (trait default + DefaultFetcher override)

- Default: fetches normally then saves `content` as bytes
- `DefaultFetcher` override: **skips binary rejection**, streams raw bytes, saves through saver

### Schema gating

- `ToolBuilder::enable_save_to_file(bool)` — disabled by default
- `Tool::input_schema()` removes `save_to_file` property when disabled
- `Tool::description()` / `Tool::llmtxt()` — composable fragments, save_to_file sections only when enabled

### Error variants

- `FetchError::SaveError(String)` — file save failed
- `FetchError::SaverNotAvailable` — no saver provided or feature disabled

## Everruns integration — implementation

In `crates/core/src/capabilities/web_fetch.rs`:

### 1. `SessionFileSaver` adapter

Bridges fetchkit's `FileSaver` to everruns' `SessionFileStore`:

```rust
use crate::traits::SessionFileStore;
use crate::session_types::SessionId;
use fetchkit::file_saver::{FileSaver, FileSaveError, SaveResult};
use base64::Engine as _;

struct SessionFileSaver {
    file_store: Arc<dyn SessionFileStore>,
    session_id: SessionId,
}

#[async_trait]
impl FileSaver for SessionFileSaver {
    async fn save(&self, path: &str, bytes: &[u8]) -> Result<SaveResult, FileSaveError> {
        // Binary detection: use base64 for non-UTF-8 content
        let (content, encoding) = match std::str::from_utf8(bytes) {
            Ok(text) => (text.to_string(), "text"),
            Err(_) => {
                let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
                (encoded, "base64")
            }
        };

        let file = self.file_store
            .write_file(self.session_id, path, &content, encoding)
            .await
            .map_err(|e| FileSaveError::Other(e.to_string()))?;

        Ok(SaveResult {
            path: file.path,
            bytes_written: bytes.len() as u64,
        })
    }
}
```

Key decisions:
- UTF-8 check determines encoding (not content-type heuristic) — correct for all cases
- Uses `base64::STANDARD` for binary, raw string for text
- Path normalization handled by WebFetchTool before passing to saver (strips `/workspace` prefix)

### 2. `WebFetchTool` changes

- `requires_context()` returns `true` (needs file_store from ToolContext)
- `execute()` handles non-save requests (backward compat, no context needed)
- `execute_with_context()` handles `save_to_file`:
  1. Parse arguments including `save_to_file`
  2. If no `save_to_file` → delegate to `execute()` (existing path)
  3. If `save_to_file` set → construct `SessionFileSaver`, call `fetchkit::Tool::execute_with_saver`
  4. Normalize path (strip `/workspace` prefix), add it back in response

- `WebFetchCapability` uses `fetchkit::Tool::builder().enable_save_to_file(true)` for schema/description/llmtxt
- `parameters_schema()` uses the fetchkit Tool with save_to_file enabled

### 3. fetchkit `Tool` instance

`WebFetchTool` holds a `fetchkit::Tool` (configured via builder) instead of raw `FetchOptions`:

```rust
pub struct WebFetchTool {
    fetchkit_tool: fetchkit::Tool,
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self {
            fetchkit_tool: fetchkit::Tool::builder()
                .enable_save_to_file(true)
                .build(),
        }
    }
}
```

This lets `WebFetchTool` delegate schema/description/execution to fetchkit's `Tool`.

## Future: `FilesSaver` (archive extraction)

Not in fetchkit 0.1.3. Design sketch:

```rust
#[async_trait]
pub trait FilesSaver: FileSaver {
    async fn save_and_extract(
        &self,
        path: &str,
        bytes: &[u8],
        content_type: Option<&str>,
    ) -> Result<ExtractResult, FileSaveError>;
}
```

- Separate trait, consumers opt-in
- Archive detection by content-type + magic bytes
- Formats: zip, tar.gz, tar
- `FetchRequest::extract: Option<bool>` field
- `FetchResponse::extracted_files: Option<Vec<String>>` field
