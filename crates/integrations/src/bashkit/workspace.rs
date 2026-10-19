use everruns_contracts::runtime::session_files::SessionFileSystem;

/// Resolve the shell working directory and `WORKSPACE` env value from the file
/// store's namespace (EVE-660).
///
/// The file store ([`MountFs`](crate::bashkit::mount_fs::MountFs)) is the single path
/// authority: `working_dir` is resolved through it to an absolute path in the
/// same namespace the file tools use, so a model that learns a path from
/// `read_file` can pass it straight to `cd`. With no `working_dir`, the shell
/// starts at the store's display root (`/workspace` for the mounted stores used
/// by agent execution). The tuple is `(cwd, workspace_env)`.
pub(super) fn resolve_shell_workspace(
    store: &dyn SessionFileSystem,
    working_dir_arg: Option<&str>,
) -> (String, String) {
    let workspace_env = store.display_root();
    let cwd = match working_dir_arg {
        Some(arg) => store.resolve_path(arg),
        None => workspace_env.clone(),
    };
    (cwd, workspace_env)
}
