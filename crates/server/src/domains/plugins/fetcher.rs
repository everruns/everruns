// Plugin source fetcher — seam between marketplace source types and the
// PluginFileSet compiler input.
//
// v1 supports:
//   local_path  — reads a directory on disk (dev/test only)
//   github      — downloads a pinned tarball from codeload.github.com,
//                 extracts the plugin subdirectory in-memory
//   url         — direct git URL plugin sources are not supported in v1;
//                 marketplace sync from a URL source is handled in commands.rs

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::kernel_imports::{EgressRequest, EgressRequestKind, EgressService};
use everruns_core::plugins::PluginFileSet;
use everruns_core::plugins::{MAX_PLUGIN_FILE_BYTES, MAX_PLUGIN_FILES, MAX_PLUGIN_TOTAL_BYTES};
use everruns_provider::url_validation::validate_url_dns_pinned;

/// Cap on the total unpacked tarball bytes before applying plugin limits.
/// GitHub only provides whole-repository archives for relative marketplace
/// sources. 128 MiB admits the first-party repository with bounded headroom
/// while the much smaller plugin-specific limits still apply to retained data.
const MAX_TARBALL_UNPACK_BYTES: usize = 128 * 1024 * 1024;

/// Independent iteration cap for archives dominated by zero-length entries.
const MAX_TARBALL_ENTRIES: usize = 100_000;

/// Cap on the compressed tarball download itself (TM-PLUGIN-004). Enforced
/// while streaming so a hostile endpoint cannot buffer unbounded bytes.
const MAX_TARBALL_DOWNLOAD_BYTES: usize = 64 * 1024 * 1024; // 64 MB

/// 30-second timeout for tarball downloads.
const TARBALL_TIMEOUT_MS: u64 = 30_000;

/// A resolved plugin file set ready for compilation.
#[derive(Debug)]
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
    /// Owner/repo for the marketplace (only for `github`).
    pub marketplace_github_repo: Option<String>,
    /// Pinned SHA from the marketplace's last sync (set for `github` marketplaces).
    pub marketplace_last_synced_sha: Option<String>,
}

/// Fetch a plugin's file set from its resolved source.
///
/// # Errors
///
/// Returns an error string when fetching is not supported for the source type
/// or when the source cannot be read.
pub async fn fetch_plugin(
    source: &PluginSource,
    egress: &Arc<dyn EgressService>,
) -> Result<FetchedPluginFileSet, String> {
    match source.marketplace_source_type.as_str() {
        "local_path" => fetch_local_path(source),
        "github" => fetch_github(source, egress).await,
        "url" => Err("url marketplace plugin sources are not supported in v1; \
             only github and local_path marketplaces support plugin fetch"
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

/// Fetch a plugin from a GitHub marketplace.
///
/// For `./subdir` (relative) sources: download the repo tarball pinned at the
/// marketplace's `last_synced_sha`, extract the plugin subdirectory.
/// For absolute `github:owner/repo` sources (future): not yet supported.
async fn fetch_github(
    source: &PluginSource,
    egress: &Arc<dyn EgressService>,
) -> Result<FetchedPluginFileSet, String> {
    let repo = source
        .marketplace_github_repo
        .as_deref()
        .ok_or("github marketplace is missing repo configuration")?;

    let sha = source
        .marketplace_last_synced_sha
        .as_deref()
        .ok_or("github marketplace has not been synced yet; run POST .../sync first")?;

    // Validate repo format.
    validate_github_repo_slug(repo)?;

    let (owner, repo_name) = split_repo(repo)?;

    // Determine the plugin subdirectory within the tarball.
    // Catalog `source` values are relative paths like `./my-plugin` or `my-plugin`.
    let plugin_subdir = source
        .source_value
        .trim_start_matches("./")
        .trim_start_matches('/');

    // Download the tarball pinned at the synced SHA.
    let tarball_url = format!("https://codeload.github.com/{owner}/{repo_name}/tar.gz/{sha}");

    let tarball_bytes = download_bytes(
        &tarball_url,
        egress,
        TARBALL_TIMEOUT_MS,
        MAX_TARBALL_DOWNLOAD_BYTES,
        &[],
    )
    .await?;

    // Extract the plugin subdirectory from the tarball.
    // GitHub tarball entries are prefixed `{repo_name}-{sha}/`.
    let tarball_prefix = format!("{repo_name}-{sha}/");
    let plugin_prefix = if plugin_subdir.is_empty() {
        tarball_prefix.clone()
    } else {
        format!("{tarball_prefix}{plugin_subdir}/")
    };

    let dir_name = if plugin_subdir.is_empty() {
        repo_name.to_string()
    } else {
        // Use the last path component as the dir name.
        plugin_subdir
            .split('/')
            .next_back()
            .unwrap_or(plugin_subdir)
            .to_string()
    };

    let file_map = extract_tarball_subdir(&tarball_bytes, &plugin_prefix, &tarball_prefix)?;

    let file_set = PluginFileSet::from_map(dir_name, file_map)
        .map_err(|e| format!("tarball extraction produced an invalid plugin file set: {e}"))?;

    Ok(FetchedPluginFileSet {
        file_set,
        resolved_sha: Some(sha.to_string()),
    })
}

/// Download raw bytes via the egress service with a timeout.
///
/// `max_bytes` is enforced while streaming (TM-PLUGIN-004): the body is
/// dropped and an error returned as soon as the cap is crossed, so a hostile
/// endpoint cannot buffer unbounded bytes into server memory.
pub(crate) async fn download_bytes(
    url: &str,
    egress: &Arc<dyn EgressService>,
    timeout_ms: u64,
    max_bytes: usize,
    headers: &[(&str, String)],
) -> Result<Vec<u8>, String> {
    use futures::StreamExt;

    // DNS-pin the connection to close the TOCTOU window (TM-TOOL-018).
    let (parsed_url, pinned_addrs) = validate_url_dns_pinned(url)
        .await
        .map_err(|e| format!("URL validation failed for '{url}': {e}"))?;

    let host = parsed_url.host_str().unwrap_or("").to_string();

    let mut req = EgressRequest::new("GET", url, EgressRequestKind::Other("plugin_fetch".into()))
        .timeout_ms(timeout_ms);

    for (name, value) in headers {
        req = req.header(*name, value.clone());
    }

    if !pinned_addrs.is_empty() {
        req = req.pinned_addrs(host, pinned_addrs);
    }

    let resp = egress
        .send_stream(req)
        .await
        .map_err(|e| format!("HTTP fetch failed for '{url}': {e}"))?;

    if resp.status < 200 || resp.status >= 300 {
        return Err(format!(
            "HTTP fetch for '{url}' returned status {}",
            resp.status
        ));
    }

    let mut body: Vec<u8> = Vec::new();
    let mut stream = resp.body;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("HTTP body read failed for '{url}': {e}"))?;
        if body.len() + chunk.len() > max_bytes {
            return Err(format!(
                "response from '{url}' exceeds the {max_bytes}-byte limit"
            ));
        }
        body.extend_from_slice(&chunk);
    }

    Ok(body)
}

/// Extract files under `plugin_prefix` from a `.tar.gz` byte slice.
///
/// Returns a map of relative path → bytes (relative to `plugin_prefix`).
/// `tarball_prefix` is the top-level directory prefix in the archive
/// (e.g. `{repo}-{sha}/`) used to scope containment checks.
fn extract_tarball_subdir(
    tarball_bytes: &[u8],
    plugin_prefix: &str,
    tarball_prefix: &str,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    extract_tarball_subdir_with_limits(
        tarball_bytes,
        plugin_prefix,
        tarball_prefix,
        MAX_TARBALL_UNPACK_BYTES,
        MAX_TARBALL_ENTRIES,
    )
}

fn extract_tarball_subdir_with_limits(
    tarball_bytes: &[u8],
    plugin_prefix: &str,
    tarball_prefix: &str,
    max_unpack_bytes: usize,
    max_entries: usize,
) -> Result<BTreeMap<String, Vec<u8>>, String> {
    use flate2::read::GzDecoder;
    use tar::Archive;

    let decoder = GzDecoder::new(tarball_bytes);
    let mut archive = Archive::new(decoder);

    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut plugin_total_bytes: usize = 0;
    let mut archive_unpacked_bytes: u64 = 0;
    let mut archive_entries: usize = 0;

    let entries = archive
        .entries()
        .map_err(|e| format!("failed to read tarball entries: {e}"))?;

    for entry_result in entries {
        let mut entry = entry_result.map_err(|e| format!("failed to read tarball entry: {e}"))?;

        let header = entry.header();

        // THREAT[TM-PLUGIN-004]: Bound both decompressed bytes and entry
        // iteration before filtering to the requested plugin subdirectory.
        archive_entries = archive_entries
            .checked_add(1)
            .filter(|count| *count <= max_entries)
            .ok_or_else(|| {
                format!("tarball contains more than {max_entries} entries (extraction limit)")
            })?;

        let entry_size = header
            .size()
            .map_err(|e| format!("cannot read entry size: {e}"))?;
        let entry_unpacked = 512u64.saturating_add(entry_size.div_ceil(512).saturating_mul(512));
        archive_unpacked_bytes = archive_unpacked_bytes
            .checked_add(entry_unpacked)
            .filter(|total| *total <= max_unpack_bytes as u64)
            .ok_or_else(|| {
                format!("tarball unpacked size exceeds {max_unpack_bytes} bytes (extraction limit)")
            })?;

        // Reject symlinks and hard links.
        let entry_type = header.entry_type();
        if entry_type.is_symlink() || entry_type.is_hard_link() {
            // Silently skip — do not error, just drop the entry.
            continue;
        }

        // Only process regular files.
        if !entry_type.is_file() {
            continue;
        }

        // Safe: a single entry larger than the archive budget would have
        // already tripped the check above, so this fits in usize.
        let file_size = entry_size as usize;

        let raw_path = entry
            .path()
            .map_err(|e| format!("invalid entry path in tarball: {e}"))?;
        let path_str = raw_path
            .to_str()
            .ok_or("tarball entry path is not valid UTF-8")?
            .to_string();

        // Only include files inside the plugin subdirectory.
        let relative_path = match path_str.strip_prefix(plugin_prefix) {
            Some(rel) => rel.to_string(),
            None => continue,
        };

        if relative_path.is_empty() {
            continue; // Skip the directory entry itself.
        }

        // Reject path traversal.
        for component in std::path::Path::new(&relative_path).components() {
            if component == std::path::Component::ParentDir {
                return Err(format!(
                    "path traversal detected in tarball: '{relative_path}'"
                ));
            }
        }

        // Reject paths escaping the tarball top-level prefix.
        let full_path = format!("{tarball_prefix}{relative_path}");
        if !full_path.starts_with(tarball_prefix) {
            return Err(format!(
                "tarball entry '{full_path}' escapes expected prefix — rejected for security"
            ));
        }

        // File count cap.
        if files.len() >= MAX_PLUGIN_FILES {
            return Err(format!(
                "plugin directory in tarball contains more than {MAX_PLUGIN_FILES} files"
            ));
        }

        if file_size > MAX_PLUGIN_FILE_BYTES {
            return Err(format!(
                "plugin file '{relative_path}' is {file_size} bytes, exceeding the {MAX_PLUGIN_FILE_BYTES}-byte limit"
            ));
        }

        plugin_total_bytes = plugin_total_bytes
            .checked_add(file_size)
            .filter(|total| *total <= MAX_PLUGIN_TOTAL_BYTES)
            .ok_or_else(|| {
                format!("plugin total size from tarball exceeds {MAX_PLUGIN_TOTAL_BYTES} bytes")
            })?;

        let mut content = Vec::with_capacity(file_size);
        std::io::Read::read_to_end(&mut entry, &mut content)
            .map_err(|e| format!("failed to read tarball entry '{relative_path}': {e}"))?;

        files.insert(relative_path, content);
    }

    if files.is_empty() {
        return Err(format!(
            "no files found under plugin path '{plugin_prefix}' in tarball"
        ));
    }

    Ok(files)
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

/// Validate `owner/repo` slug format.
pub(crate) fn validate_github_repo_slug(repo: &str) -> Result<(), String> {
    let re =
        regex::Regex::new(r"^[A-Za-z0-9_.\-]+/[A-Za-z0-9_.\-]+$").expect("static regex is valid");
    if !re.is_match(repo) {
        return Err(format!(
            "invalid GitHub repo slug '{repo}'; expected 'owner/repo' with alphanumeric, hyphens, underscores, or dots"
        ));
    }
    Ok(())
}

fn split_repo(repo: &str) -> Result<(&str, &str), String> {
    let parts: Vec<&str> = repo.splitn(2, '/').collect();
    if parts.len() != 2 {
        return Err(format!("invalid repo slug '{repo}': expected 'owner/repo'"));
    }
    Ok((parts[0], parts[1]))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------------------------------------------------------------------------
    // Helper: build an in-memory .tar.gz with the given file map.
    // `root_prefix` is the top-level directory prefix (e.g. `myrepo-abc123/`).
    // ---------------------------------------------------------------------------
    pub fn build_tarball(
        root_prefix: &str,
        files: &[(&str, &[u8])],
        symlinks: &[(&str, &str)], // (name, target)
    ) -> Vec<u8> {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use tar::{Builder, EntryType, Header};

        let buf = Vec::new();
        let enc = GzEncoder::new(buf, Compression::fast());
        let mut ar = Builder::new(enc);

        for (path, content) in files {
            let full_path = format!("{root_prefix}{path}");
            let mut header = Header::new_gnu();
            header.set_path(&full_path).unwrap();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            ar.append(&header, *content).unwrap();
        }

        for (name, target) in symlinks {
            let full_path = format!("{root_prefix}{name}");
            let mut header = Header::new_gnu();
            header.set_entry_type(EntryType::Symlink);
            header.set_path(&full_path).unwrap();
            header.set_link_name(target).unwrap();
            header.set_size(0);
            header.set_mode(0o777);
            header.set_cksum();
            ar.append(&header, &b""[..]).unwrap();
        }

        let encoder = ar.into_inner().unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn extract_tarball_basic() {
        let tarball = build_tarball(
            "myrepo-abc123/",
            &[
                (
                    "plugins/my-plugin/.claude-plugin/plugin.json",
                    br#"{"name":"my-plugin","version":"1.0.0"}"#,
                ),
                ("plugins/my-plugin/README.md", b"# My Plugin"),
            ],
            &[],
        );

        let files = extract_tarball_subdir(
            &tarball,
            "myrepo-abc123/plugins/my-plugin/",
            "myrepo-abc123/",
        )
        .unwrap();

        assert!(files.contains_key(".claude-plugin/plugin.json"));
        assert!(files.contains_key("README.md"));
    }

    #[test]
    fn extract_tarball_rejects_symlinks() {
        let tarball = build_tarball(
            "myrepo-abc123/",
            &[("plugins/my-plugin/hello.md", b"hello")],
            &[("plugins/my-plugin/link.md", "/etc/passwd")],
        );

        // Symlinks are silently skipped — they should NOT appear in the result.
        let files = extract_tarball_subdir(
            &tarball,
            "myrepo-abc123/plugins/my-plugin/",
            "myrepo-abc123/",
        )
        .unwrap();

        assert!(!files.contains_key("link.md"), "symlink should be skipped");
        assert!(files.contains_key("hello.md"));
    }

    #[test]
    fn extract_tarball_rejects_oversized_file() {
        let big = vec![b'x'; MAX_PLUGIN_FILE_BYTES + 1];
        let tarball = build_tarball(
            "myrepo-abc123/",
            &[("plugins/my-plugin/big.bin", big.as_slice())],
            &[],
        );

        let err = extract_tarball_subdir(
            &tarball,
            "myrepo-abc123/plugins/my-plugin/",
            "myrepo-abc123/",
        )
        .unwrap_err();

        assert!(
            err.contains("exceeding the"),
            "expected size error, got: {err}"
        );
    }

    #[test]
    fn extract_tarball_rejects_oversized_skipped_file() {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use std::io::Read as _;
        use tar::{Builder, Header};

        // Build the archive with the oversized entry *streamed* (via
        // `io::repeat(..).take(..)`) rather than materializing the full
        // uncompressed payload in a Vec, so this test stays cheap on memory.
        let enc = GzEncoder::new(Vec::new(), Compression::fast());
        let mut ar = Builder::new(enc);

        let max_unpack_bytes = 1024 * 1024;
        let big_size = max_unpack_bytes as u64 + 1;
        let mut big_header = Header::new_gnu();
        big_header
            .set_path("myrepo-abc123/outside-plugin/big.bin")
            .unwrap();
        big_header.set_size(big_size);
        big_header.set_mode(0o644);
        big_header.set_cksum();
        ar.append(&big_header, std::io::repeat(b'x').take(big_size))
            .unwrap();

        let plugin_json = br#"{"name":"my-plugin","version":"1.0.0"}"#;
        let mut plugin_header = Header::new_gnu();
        plugin_header
            .set_path("myrepo-abc123/plugins/my-plugin/.claude-plugin/plugin.json")
            .unwrap();
        plugin_header.set_size(plugin_json.len() as u64);
        plugin_header.set_mode(0o644);
        plugin_header.set_cksum();
        ar.append(&plugin_header, &plugin_json[..]).unwrap();

        let tarball = ar.into_inner().unwrap().finish().unwrap();

        let err = extract_tarball_subdir_with_limits(
            &tarball,
            "myrepo-abc123/plugins/my-plugin/",
            "myrepo-abc123/",
            max_unpack_bytes,
            MAX_TARBALL_ENTRIES,
        )
        .unwrap_err();

        assert!(
            err.contains("tarball unpacked size exceeds"),
            "expected archive-wide unpacked size error, got: {err}"
        );
    }

    #[test]
    fn extract_tarball_accepts_repo_larger_than_64_mib_when_plugin_is_small() {
        use flate2::Compression;
        use flate2::write::GzEncoder;
        use std::io::Read as _;
        use tar::{Builder, Header};

        let enc = GzEncoder::new(Vec::new(), Compression::fast());
        let mut ar = Builder::new(enc);

        let unrelated_size = 65 * 1024 * 1024;
        let mut unrelated_header = Header::new_gnu();
        unrelated_header
            .set_path("myrepo-abc123/unrelated/large-fixture.bin")
            .unwrap();
        unrelated_header.set_size(unrelated_size);
        unrelated_header.set_mode(0o644);
        unrelated_header.set_cksum();
        ar.append(
            &unrelated_header,
            std::io::repeat(b'x').take(unrelated_size),
        )
        .unwrap();

        let plugin_json = br#"{"name":"my-plugin","version":"1.0.0"}"#;
        let mut plugin_header = Header::new_gnu();
        plugin_header
            .set_path("myrepo-abc123/plugins/my-plugin/.claude-plugin/plugin.json")
            .unwrap();
        plugin_header.set_size(plugin_json.len() as u64);
        plugin_header.set_mode(0o644);
        plugin_header.set_cksum();
        ar.append(&plugin_header, &plugin_json[..]).unwrap();

        let tarball = ar.into_inner().unwrap().finish().unwrap();
        let previous_limit = 64 * 1024 * 1024;
        let previous_error = extract_tarball_subdir_with_limits(
            &tarball,
            "myrepo-abc123/plugins/my-plugin/",
            "myrepo-abc123/",
            previous_limit,
            MAX_TARBALL_ENTRIES,
        )
        .unwrap_err();
        assert_eq!(
            previous_error,
            format!("tarball unpacked size exceeds {previous_limit} bytes (extraction limit)")
        );

        let files = extract_tarball_subdir(
            &tarball,
            "myrepo-abc123/plugins/my-plugin/",
            "myrepo-abc123/",
        )
        .unwrap();

        assert_eq!(
            files.get(".claude-plugin/plugin.json"),
            Some(&plugin_json.to_vec())
        );
    }

    #[test]
    fn extract_tarball_rejects_too_many_entries_before_plugin_filtering() {
        let tarball = build_tarball(
            "myrepo-abc123/",
            &[
                ("outside/one.txt", b"one"),
                ("outside/two.txt", b"two"),
                (
                    "plugins/my-plugin/.claude-plugin/plugin.json",
                    br#"{"name":"my-plugin"}"#,
                ),
            ],
            &[],
        );

        let err = extract_tarball_subdir_with_limits(
            &tarball,
            "myrepo-abc123/plugins/my-plugin/",
            "myrepo-abc123/",
            MAX_TARBALL_UNPACK_BYTES,
            2,
        )
        .unwrap_err();

        assert!(
            err.contains("more than 2 entries"),
            "expected entry-count error, got: {err}"
        );
    }

    #[test]
    fn extract_tarball_empty_subdir_returns_error() {
        // Tarball exists but has nothing under the plugin prefix.
        let tarball = build_tarball(
            "myrepo-abc123/",
            &[("other-dir/README.md", b"unrelated")],
            &[],
        );

        let err = extract_tarball_subdir(
            &tarball,
            "myrepo-abc123/plugins/missing-plugin/",
            "myrepo-abc123/",
        )
        .unwrap_err();

        assert!(err.contains("no files found"), "got: {err}");
    }

    #[test]
    fn validate_github_repo_slug_valid() {
        assert!(validate_github_repo_slug("owner/repo").is_ok());
        assert!(validate_github_repo_slug("My-Org/my.repo_2").is_ok());
        assert!(validate_github_repo_slug("a/b").is_ok());
    }

    #[test]
    fn validate_github_repo_slug_invalid() {
        assert!(validate_github_repo_slug("no-slash").is_err());
        assert!(validate_github_repo_slug("bad/repo/extra").is_err());
        assert!(validate_github_repo_slug("../etc/passwd").is_err());
        assert!(validate_github_repo_slug("owner/repo;drop").is_err());
    }
}
