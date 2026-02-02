# Bashkit Requirements for Custom FileSystem Adapters

> **Status: IMPLEMENTED** - bashkit now exports the required types and
> `SessionFileSystemAdapter` has been implemented in `crates/core/src/capabilities/virtual_bash.rs`.
>
> **Workspace Mount**: Session files are mounted at `/workspace` in the bash environment.
> Both `virtual_bash` and `session_file_system` capabilities normalize paths, enabling
> seamless file sharing between bash commands and file system tools.

## Context

Everruns implements a custom `FileSystem` adapter that bridges bashkit to the session file store. This enables live visibility of files during bash execution - if another tool writes to the session filesystem while bash is running, those files are immediately visible.

The implementation is in `crates/core/src/capabilities/virtual_bash.rs` with `SessionFileSystemAdapter`.

## Required Exports

Add to `bashkit/src/lib.rs`:

```rust
pub use fs::{DirEntry, FileType, Metadata};  // ADD these
```

### Types Needed

**1. `FileType`** (from `fs/traits.rs:82-90`)
```rust
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileType {
    File,
    Directory,
    Symlink,
}
```

Required for:
- Implementing `stat()` return value
- Checking entry types in `read_dir()` results

**2. `Metadata`** (from `fs/traits.rs:54-67`)
```rust
#[derive(Debug, Clone)]
pub struct Metadata {
    pub file_type: FileType,
    pub size: u64,
    pub mode: u32,
    pub modified: SystemTime,
    pub created: SystemTime,
}
```

Required for:
- `stat(&self, path: &Path) -> Result<Metadata>`
- Building `DirEntry` instances

**3. `DirEntry`** (from `fs/traits.rs:109-116`)
```rust
#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub metadata: Metadata,
}
```

Required for:
- `read_dir(&self, path: &Path) -> Result<Vec<DirEntry>>`

## FileSystem Trait Methods

For reference, the full trait that users need to implement:

```rust
#[async_trait]
pub trait FileSystem: Send + Sync {
    async fn read_file(&self, path: &Path) -> Result<Vec<u8>>;
    async fn write_file(&self, path: &Path, content: &[u8]) -> Result<()>;
    async fn append_file(&self, path: &Path, content: &[u8]) -> Result<()>;
    async fn mkdir(&self, path: &Path, recursive: bool) -> Result<()>;
    async fn remove(&self, path: &Path, recursive: bool) -> Result<()>;
    async fn stat(&self, path: &Path) -> Result<Metadata>;           // needs Metadata
    async fn read_dir(&self, path: &Path) -> Result<Vec<DirEntry>>;  // needs DirEntry
    async fn exists(&self, path: &Path) -> Result<bool>;
    async fn rename(&self, from: &Path, to: &Path) -> Result<()>;
    async fn copy(&self, from: &Path, to: &Path) -> Result<()>;
    async fn symlink(&self, target: &Path, link: &Path) -> Result<()>;
    async fn read_link(&self, path: &Path) -> Result<PathBuf>;
    async fn chmod(&self, path: &Path, mode: u32) -> Result<()>;
}
```

## Implementation Change

**File:** `crates/bashkit/src/lib.rs`

**Current (line 29):**
```rust
pub use fs::{FileSystem, InMemoryFs, MountableFs, OverlayFs};
```

**Required:**
```rust
pub use fs::{DirEntry, FileSystem, FileType, InMemoryFs, Metadata, MountableFs, OverlayFs};
```

## Use Case: Session FileSystem Adapter

Once exported, Everruns can implement:

```rust
use bashkit::{DirEntry, FileSystem, FileType, Metadata, Result};

pub struct SessionFileSystemAdapter {
    session_id: SessionId,
    store: Arc<dyn SessionFileStore>,
}

#[async_trait]
impl FileSystem for SessionFileSystemAdapter {
    async fn read_file(&self, path: &Path) -> Result<Vec<u8>> {
        // Delegate to SessionFileStore::read_file
    }

    async fn write_file(&self, path: &Path, content: &[u8]) -> Result<()> {
        // Delegate to SessionFileStore::write_file
    }

    async fn stat(&self, path: &Path) -> Result<Metadata> {
        // Build Metadata from SessionFile info
        Ok(Metadata {
            file_type: FileType::File,
            size: content.len() as u64,
            mode: 0o644,
            modified: SystemTime::now(),
            created: SystemTime::now(),
        })
    }

    async fn read_dir(&self, path: &Path) -> Result<Vec<DirEntry>> {
        // Map SessionFileStore::list_directory to Vec<DirEntry>
    }

    // ... other methods
}
```

## Benefits

1. **Live file visibility** - Files written by other tools during bash execution are immediately visible
2. **No sync overhead** - Eliminates pre/post execution sync of entire filesystem
3. **Memory efficiency** - Files read on-demand instead of loading all into memory
4. **Consistency** - Single source of truth for file state

## Backward Compatibility

This is a purely additive change - only adds new exports. No breaking changes.

## Priority

Medium - Current sync-based approach works for most use cases. This optimization matters for:
- Long-running bash scripts
- Concurrent tool execution
- Large session filesystems
