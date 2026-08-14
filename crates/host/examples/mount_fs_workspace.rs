// Mount-based workspace filesystem (EVE-660).
//
// Demonstrates `MountFs`: the agent sees ONE namespace where `/workspace` is a
// mount point and the default current working directory — not a magic prefix
// re-implemented in every store. The same resolver works over the in-memory VFS
// and over a real host directory, and a worktree switch repoints every surface
// at once while preserving a stable agent-facing display identity.
//
// Run it:
//   cargo run -p everruns-host --example mount_fs_workspace
//
// See `knowledge/runtime-resources/file-store.md` for the contract.

use everruns_core::{MountFs, WorkspaceRootSet, session_files::SessionFileSystem};
use everruns_host::{InMemorySessionFileStore, RealDiskFileStore, multi_root_file_system};
use everruns_provider::typed_id::SessionId;
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let session = SessionId::new();

    // ---------------------------------------------------------------------
    // 1. One namespace + cwd, over the in-memory VFS.
    // ---------------------------------------------------------------------
    let backend: Arc<dyn SessionFileSystem> = Arc::new(InMemorySessionFileStore::new());
    let fs = MountFs::wrap(backend);

    // Write the file using the `/workspace` mount view.
    fs.write_file(session, "/workspace/src/lib.rs", "fn main() {}", "text")
        .await?;

    // The same file is reachable three ways — all resolve to one backend key:
    //   * the /workspace mount view
    //   * the backend-native absolute path
    //   * a path relative to the cwd (which defaults to /workspace)
    for input in ["/workspace/src/lib.rs", "/src/lib.rs", "src/lib.rs"] {
        let file = fs.read_file(session, input).await?.expect("file exists");
        assert_eq!(file.content.as_deref(), Some("fn main() {}"));
        // The store keys it at /src/lib.rs; `/workspace` is never in the keyspace.
        assert_eq!(file.path, "/src/lib.rs");
        println!("read {input:<24} -> backend key {} ✓", file.path);
    }

    // The in-memory backend's own display identity is /workspace.
    assert_eq!(fs.display_root(), "/workspace");
    assert_eq!(fs.display_path("/src/lib.rs"), "/workspace/src/lib.rs");
    println!(
        "display_root = {}, display_path(/src/lib.rs) = {}",
        fs.display_root(),
        fs.display_path("/src/lib.rs")
    );

    // ---------------------------------------------------------------------
    // 2. The same resolver over a real host directory, across a worktree switch.
    // ---------------------------------------------------------------------
    let worktree_a = tempfile::tempdir()?;
    let worktree_b = tempfile::tempdir()?;

    let disk = Arc::new(RealDiskFileStore::new(worktree_a.path())?);
    let host_fs = MountFs::wrap(disk.clone());

    host_fs
        .write_file(session, "/workspace/notes.md", "from worktree A", "text")
        .await?;
    let on_disk_a = std::fs::read_to_string(disk.root().join("notes.md"))?;
    println!("\nworktree A root: {}", disk.root().display());
    println!("  /workspace/notes.md -> on disk: {on_disk_a:?}");
    assert_eq!(on_disk_a, "from worktree A");

    // The embedder switches worktrees: repoint the host root once.
    disk.set_host_root(worktree_b.path())?;
    host_fs
        .write_file(session, "/workspace/notes.md", "from worktree B", "text")
        .await?;
    let on_disk_b = std::fs::read_to_string(disk.root().join("notes.md"))?;
    println!("worktree B root: {}", disk.root().display());
    println!("  /workspace/notes.md -> on disk: {on_disk_b:?}");
    assert_eq!(on_disk_b, "from worktree B");

    // MountFs hides the backing root from the agent-facing namespace, while the
    // direct backend remains available to host-side integrations.
    assert_eq!(host_fs.display_root(), "/workspace");
    assert_eq!(disk.display_root(), disk.root().display().to_string());
    println!(
        "\nmodel-facing root remains host-agnostic: `{}` ✓",
        host_fs.display_root()
    );

    // ---------------------------------------------------------------------
    // 3. Multiple host roots in one session namespace.
    // ---------------------------------------------------------------------
    let primary = tempfile::tempdir()?;
    let backend = tempfile::tempdir()?;
    let roots = WorkspaceRootSet::new(
        primary.path(),
        [("backend".to_string(), backend.path().to_path_buf())],
    )?;
    let multi_root_fs = multi_root_file_system(&roots)?;

    multi_root_fs
        .write_file(session, "/workspace/README.md", "primary", "text")
        .await?;
    multi_root_fs
        .write_file(
            session,
            "/workspace/roots/backend/Cargo.toml",
            "backend",
            "text",
        )
        .await?;
    assert_eq!(
        std::fs::read_to_string(primary.path().join("README.md"))?,
        "primary"
    );
    assert_eq!(
        std::fs::read_to_string(backend.path().join("Cargo.toml"))?,
        "backend"
    );
    println!(
        "multi-root write: /workspace -> {}, /workspace/roots/backend -> {}",
        primary.path().display(),
        backend.path().display()
    );

    println!("\nmount_fs_workspace example OK");
    Ok(())
}
