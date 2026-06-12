// Plugin source fetcher — seam between marketplace source types and the
// PluginFileSet compiler input.
//
// v1 supports:
//   local_path  — reads a directory on disk (dev/test only)
//   github      — stub (tar/gzip deps not yet available) → returns an error
//   url         — stub (requires additional review of egress layer for binary fetch)
//
// The `PluginSourceFetcher` trait is the extension seam: implement for GitHub
// tarball fetch when the `tar` / `flate2` workspace deps are confirmed.

#[cfg(test)]
use std::collections::BTreeMap;

use everruns_core::plugins::PluginFileSet;

/// A resolved plugin file set ready for compilation.
pub struct FetchedPluginFileSet {
    pub file_set: PluginFileSet,
    /// SHA or version string representing what was fetched, if known.
    pub resolved_sha: Option<String>,
}

/// Source description for a plugin entry in a marketplace catalog.
pub struct PluginSource {
    /// The relative or absolute source specifier from the catalog.
    pub source_value: String,
    /// Marketplace source type: `local_path`, `github`, `url`.
    pub marketplace_source_type: String,
    /// Absolute local path to the marketplace root (only for `local_path`).
    pub marketplace_local_path: Option<String>,
}

/// Fetch a plugin's file set from its resolved source.
///
/// # Errors
///
/// Returns an error string when fetching is not supported for the source type
/// or when the local path cannot be read.
pub fn fetch_plugin(source: &PluginSource) -> Result<FetchedPluginFileSet, String> {
    match source.marketplace_source_type.as_str() {
        "local_path" => fetch_local_path(source),
        "github" => Err("github source type is not yet supported in v1; \
             use a local_path marketplace for development"
            .to_string()),
        "url" => Err("url source type is not yet supported in v1; \
             use a local_path marketplace for development"
            .to_string()),
        other => Err(format!("unknown marketplace source type: {other}")),
    }
}

fn fetch_local_path(source: &PluginSource) -> Result<FetchedPluginFileSet, String> {
    let marketplace_root = source.marketplace_local_path.as_deref().ok_or_else(|| {
        "local_path marketplace is missing a local path configuration".to_string()
    })?;

    // Resolve the plugin path relative to the marketplace root.
    let plugin_path = resolve_local_plugin_path(marketplace_root, &source.source_value)?;

    let file_set = PluginFileSet::from_dir(&plugin_path).map_err(|e| {
        format!(
            "failed to load plugin from {}: {}",
            plugin_path.display(),
            e
        )
    })?;

    Ok(FetchedPluginFileSet {
        file_set,
        resolved_sha: None, // local_path has no SHA
    })
}

fn resolve_local_plugin_path(
    marketplace_root: &str,
    source_value: &str,
) -> Result<std::path::PathBuf, String> {
    let root = std::path::Path::new(marketplace_root);

    // Relative path (e.g. `./microsoft-docs` or `microsoft-docs`).
    let relative = source_value
        .trim_start_matches("./")
        .trim_start_matches('/');

    let plugin_path = root.join(relative);

    // Safety: canonicalize and verify the result is within the marketplace root.
    let canonical_root = root
        .canonicalize()
        .map_err(|e| format!("cannot canonicalize marketplace root {marketplace_root}: {e}"))?;
    let canonical_plugin = plugin_path.canonicalize().map_err(|e| {
        format!(
            "cannot canonicalize plugin path {}: {e}",
            plugin_path.display()
        )
    })?;

    if !canonical_plugin.starts_with(&canonical_root) {
        return Err(format!(
            "plugin path {} escapes marketplace root — rejected for security",
            canonical_plugin.display()
        ));
    }

    Ok(canonical_plugin)
}

/// Build an in-memory `PluginFileSet` from a raw byte map.
///
/// Useful for unit tests that construct plugin directories programmatically.
#[cfg(test)]
pub fn file_set_from_map(dir_name: &str, files: BTreeMap<String, Vec<u8>>) -> PluginFileSet {
    PluginFileSet {
        files,
        dir_name: dir_name.to_string(),
    }
}
