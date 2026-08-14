//! Open Knowledge Format (OKF) import. See knowledge/runtime-resources/okf-adoption.md.
//!
//! OKF is a vendor-neutral interchange format: a bundle is a directory tree of
//! markdown files with YAML frontmatter. This module parses a bundle and
//! upserts its concept documents into a Knowledge Base as entries.
//!
//! The parsing and mapping logic here is pure and unit-tested without a
//! database; the idempotent upsert runs against the storage backend.

use super::types::{CreateKnowledgeEntryRow, KnowledgeEntryRow, UpdateKnowledgeEntry};
use super::{MAX_ENTRY_BODY_BYTES, MAX_ENTRY_TAGS};
use crate::storage::StorageBackend;
use anyhow::{Context, Result};
use everruns_provider::typed_id::KnowledgeEntryId;
use std::collections::HashMap;
use std::io::Read;

/// Reserved filenames in an OKF bundle: never imported as concept documents.
const RESERVED_FILES: &[&str] = &["index.md", "log.md"];

/// Reserved tag prefixes used to round-trip OKF fidelity that has no first-class
/// column yet. Excluded from the user-facing tag budget and hidden in the UI.
pub const OKF_TYPE_TAG_PREFIX: &str = "okf:type=";
pub const OKF_PATH_TAG_PREFIX: &str = "okf:path=";

/// Max length of an OKF `resource` URI accepted on import. Mirrors the CRUD
/// `MAX_RESOURCE_LEN` so untrusted bundles can't store pathologically large
/// values (DB bloat + oversized API/tool responses).
const MAX_RESOURCE_LEN: usize = 2048;
/// Per-file and total decompressed caps for tarball bundles, defending against
/// gzip bombs (a small gzip can expand to huge data).
const MAX_BUNDLE_FILE_BYTES: u64 = 1024 * 1024; // 1 MiB per file
const MAX_BUNDLE_UNCOMPRESSED_BYTES: u64 = 64 * 1024 * 1024; // 64 MiB total

/// True if a bundle-relative path is unsafe (absolute or parent traversal).
/// Applied to both tarball entries and inline `files[].path` so a malicious
/// `okf:path` can't later be re-emitted as a traversing tar entry on export.
fn is_unsafe_path(path: &str) -> bool {
    path.starts_with('/') || path.split('/').any(|c| c == "..")
}

/// Map a free-form OKF `type` string onto our closed `kind` enum.
/// Case-insensitive substring match; defaults to `note`. See knowledge/runtime-resources/okf-adoption.md.
pub fn map_type_to_kind(okf_type: &str) -> &'static str {
    let t = okf_type.to_lowercase();
    let has = |needle: &str| t.contains(needle);
    if has("table") || has("dataset") || has("view") {
        "table"
    } else if has("metric") || has("business") || has("kpi") || has("definition") {
        "business"
    } else if has("query") || has("sql") {
        "query"
    } else if has("playbook") || has("runbook") || has("procedure") {
        "runbook"
    } else {
        "note"
    }
}

/// A single OKF concept document parsed from a bundle file.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedOkfDoc {
    /// Bundle-relative path, e.g. `tables/orders.md`. Used as the fallback
    /// idempotency key and recorded as an `okf:path=` tag.
    pub source_path: String,
    pub title: String,
    pub body: String,
    pub kind: String,
    /// Raw OKF `type` string, preserved for faithful export.
    pub raw_type: String,
    pub resource: Option<String>,
    pub tags: Vec<String>,
}

/// Split a markdown file into (optional frontmatter YAML, body).
/// Frontmatter is a leading `---`-delimited block.
fn split_frontmatter(content: &str) -> (Option<&str>, &str) {
    let trimmed = content.trim_start_matches('\u{feff}');
    let Some(rest) = trimmed.strip_prefix("---") else {
        return (None, content);
    };
    // The opening fence must be followed by a newline.
    let Some(rest) = rest
        .strip_prefix('\n')
        .or_else(|| rest.strip_prefix("\r\n"))
    else {
        return (None, content);
    };
    // Find the closing fence at the start of a line.
    for marker in ["\n---\n", "\n---\r\n", "\r\n---\r\n"] {
        if let Some(idx) = rest.find(marker) {
            let fm = &rest[..idx];
            let body = &rest[idx + marker.len()..];
            return (Some(fm), body);
        }
    }
    // Closing fence may be the final line with no trailing newline.
    if let Some(fm) = rest
        .strip_suffix("\n---")
        .or_else(|| rest.strip_suffix("\r\n---"))
    {
        return (Some(fm), "");
    }
    (None, content)
}

/// True if a bundle-relative path is a reserved OKF file (index.md / log.md).
pub fn is_reserved_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    RESERVED_FILES.contains(&name)
}

/// Parse one bundle file into a concept document.
///
/// Returns `Ok(None)` for reserved files (index.md / log.md), which carry no
/// entry. Returns an error only when the file claims to be a concept document
/// (non-reserved `.md`) but lacks a parseable non-empty `type`, matching OKF
/// conformance: every non-reserved `.md` must have frontmatter with a `type`.
pub fn parse_okf_document(path: &str, content: &str) -> Result<Option<ParsedOkfDoc>> {
    if is_reserved_file(path) {
        return Ok(None);
    }
    if is_unsafe_path(path) {
        anyhow::bail!("{path}: unsafe path (absolute or parent traversal)");
    }

    let (frontmatter, body) = split_frontmatter(content);
    let frontmatter = frontmatter.with_context(|| format!("{path}: missing YAML frontmatter"))?;
    let fm: serde_yaml::Mapping = serde_yaml::from_str(frontmatter)
        .with_context(|| format!("{path}: invalid frontmatter"))?;

    let get_str = |key: &str| -> Option<String> {
        fm.get(serde_yaml::Value::String(key.to_string()))
            .and_then(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };

    let raw_type = get_str("type")
        .with_context(|| format!("{path}: frontmatter is missing the required `type` field"))?;
    let kind = map_type_to_kind(&raw_type).to_string();

    // Title falls back to the OKF title, then the filename stem.
    let title = get_str("title").unwrap_or_else(|| {
        path.rsplit('/')
            .next()
            .unwrap_or(path)
            .trim_end_matches(".md")
            .replace(['-', '_'], " ")
    });

    // Prepend the description as a lead paragraph if the body lacks one; OKF
    // keeps description in frontmatter, our entries inline everything in body.
    let mut body = body.trim().to_string();
    if let Some(desc) = get_str("description")
        && !body.contains(&desc)
    {
        body = if body.is_empty() {
            desc
        } else {
            format!("{desc}\n\n{body}")
        };
    }

    // Drop an over-length resource rather than store it (OKF: degrade gracefully;
    // mirrors the CRUD MAX_RESOURCE_LEN guard).
    let resource = get_str("resource").filter(|r| r.len() <= MAX_RESOURCE_LEN);

    // User tags from the `tags` list, lowercased and de-duplicated.
    let mut tags: Vec<String> = Vec::new();
    if let Some(seq) = fm
        .get(serde_yaml::Value::String("tags".to_string()))
        .and_then(|v| v.as_sequence())
    {
        for v in seq {
            if let Some(s) = v.as_str() {
                let t = s.trim().to_lowercase();
                if !t.is_empty() && !tags.iter().any(|e| e == &t) {
                    tags.push(t);
                }
            }
        }
    }

    Ok(Some(ParsedOkfDoc {
        source_path: path.to_string(),
        title,
        body,
        kind,
        raw_type,
        resource,
        tags,
    }))
}

/// Decode a gzipped tarball into `(path, utf8-content)` pairs for `.md` files.
/// Non-markdown files and directory entries are ignored. Rejects path traversal.
pub fn decode_tar_gz_bundle(bytes: &[u8]) -> Result<Vec<(String, String)>> {
    let decoder = flate2::read::GzDecoder::new(bytes);
    let mut archive = tar::Archive::new(decoder);
    let mut out = Vec::new();
    let mut total_uncompressed: u64 = 0;
    for entry in archive.entries().context("reading tar entries")? {
        let mut entry = entry.context("reading tar entry")?;
        let path = entry
            .path()
            .context("tar entry path")?
            .to_string_lossy()
            .into_owned();
        if is_unsafe_path(&path) {
            anyhow::bail!("unsafe path in bundle: {path}");
        }
        if !path.ends_with(".md") {
            continue;
        }
        // Cap per-file and total decompressed size (defends against gzip bombs);
        // the tar header size can lie, so also bound the actual read below.
        if entry.size() > MAX_BUNDLE_FILE_BYTES {
            anyhow::bail!("bundle file {path} exceeds {MAX_BUNDLE_FILE_BYTES} bytes");
        }
        total_uncompressed = total_uncompressed.saturating_add(entry.size());
        if total_uncompressed > MAX_BUNDLE_UNCOMPRESSED_BYTES {
            anyhow::bail!("bundle decompresses to more than {MAX_BUNDLE_UNCOMPRESSED_BYTES} bytes");
        }
        let mut content = String::new();
        let mut limited = (&mut entry).take(MAX_BUNDLE_FILE_BYTES + 1);
        // Tolerate non-utf8 by skipping with no entry rather than failing the bundle.
        if limited.read_to_string(&mut content).is_ok() {
            if content.len() as u64 > MAX_BUNDLE_FILE_BYTES {
                anyhow::bail!("bundle file {path} exceeds {MAX_BUNDLE_FILE_BYTES} bytes");
            }
            // Normalize a leading `./` so paths are bundle-relative.
            let path = path.strip_prefix("./").unwrap_or(&path).to_string();
            out.push((path, content));
        }
    }
    Ok(out)
}

/// Outcome of an import run.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, utoipa::ToSchema)]
pub struct OkfImportSummary {
    pub created: usize,
    pub updated: usize,
    /// Documents skipped (e.g. oversized body); see `warnings`.
    pub skipped: usize,
    /// Entries removed because `prune` was set and they were absent from the bundle.
    pub pruned: usize,
    /// Non-fatal issues (broken docs, oversized bodies). OKF consumers degrade gracefully.
    pub warnings: Vec<String>,
}

/// The idempotency key for an entry: its `resource` if present, else its
/// recorded `okf:path` tag. Returns `None` for entries not from an OKF import.
fn import_key(entry: &KnowledgeEntryRow) -> Option<String> {
    if let Some(resource) = &entry.resource {
        return Some(format!("resource:{resource}"));
    }
    entry
        .tags
        .iter()
        .find_map(|t| t.strip_prefix(OKF_PATH_TAG_PREFIX))
        .map(|p| format!("path:{p}"))
}

/// The idempotency key for a parsed doc, matching `import_key`'s scheme.
fn doc_key(doc: &ParsedOkfDoc) -> String {
    match &doc.resource {
        Some(resource) => format!("resource:{resource}"),
        None => format!("path:{}", doc.source_path),
    }
}

/// Build the stored tag set for an imported doc: user tags plus reserved
/// `okf:type` / `okf:path` tags, capped at the DB tag-count limit.
fn build_import_tags(doc: &ParsedOkfDoc) -> Vec<String> {
    // Preserve the raw `type` verbatim (case included) so export reproduces it.
    let mut tags = vec![
        format!("{OKF_TYPE_TAG_PREFIX}{}", doc.raw_type),
        format!("{OKF_PATH_TAG_PREFIX}{}", doc.source_path),
    ];
    for t in &doc.tags {
        if tags.len() >= MAX_ENTRY_TAGS {
            break;
        }
        tags.push(t.clone());
    }
    tags
}

/// Idempotently upsert parsed OKF documents into a knowledge base. When `prune`
/// is set, previously-imported entries absent from this bundle are deleted.
pub async fn import_parsed_bundle(
    db: &StorageBackend,
    kb_internal_id: uuid::Uuid,
    docs: Vec<ParsedOkfDoc>,
    prune: bool,
) -> Result<OkfImportSummary> {
    let mut summary = OkfImportSummary::default();

    // Index existing entries by their import key for matching.
    let existing = db
        .list_knowledge_entries(kb_internal_id, None, None)
        .await?;
    let mut by_key: HashMap<String, KnowledgeEntryRow> = HashMap::new();
    for entry in &existing {
        if let Some(key) = import_key(entry) {
            by_key.insert(key, entry.clone());
        }
    }
    let mut matched_keys: std::collections::HashSet<String> = std::collections::HashSet::new();

    for doc in docs {
        if doc.body.len() > MAX_ENTRY_BODY_BYTES {
            summary.skipped += 1;
            summary
                .warnings
                .push(format!("{}: body exceeds 64 KiB; skipped", doc.source_path));
            continue;
        }
        let key = doc_key(&doc);
        matched_keys.insert(key.clone());
        let tags = build_import_tags(&doc);
        // Two reserved okf:* tags consume part of the DB tag budget; warn when
        // that forces user tags to be dropped rather than dropping silently.
        let dropped = (doc.tags.len() + 2).saturating_sub(MAX_ENTRY_TAGS);
        if dropped > 0 {
            summary.warnings.push(format!(
                "{}: {dropped} user tag(s) dropped (max {MAX_ENTRY_TAGS} tags including reserved okf:*)",
                doc.source_path
            ));
        }

        if let Some(entry) = by_key.get(&key) {
            db.update_knowledge_entry(
                kb_internal_id,
                entry.id,
                UpdateKnowledgeEntry {
                    title: Some(doc.title),
                    body: Some(doc.body),
                    kind: Some(doc.kind),
                    tags: Some(tags),
                    resource: Some(doc.resource),
                },
            )
            .await?;
            summary.updated += 1;
        } else {
            db.create_knowledge_entry(
                kb_internal_id,
                CreateKnowledgeEntryRow {
                    public_id: KnowledgeEntryId::new().to_string(),
                    title: doc.title,
                    body: doc.body,
                    kind: doc.kind,
                    tags,
                    resource: doc.resource,
                },
            )
            .await?;
            summary.created += 1;
        }
    }

    if prune {
        for (key, entry) in &by_key {
            if !matched_keys.contains(key)
                && db.delete_knowledge_entry(kb_internal_id, entry.id).await?
            {
                summary.pruned += 1;
            }
        }
    }

    Ok(summary)
}

/// Parse a decoded bundle into concept documents, collecting warnings for
/// malformed files rather than failing the whole import (OKF tolerates these).
pub fn parse_bundle(
    files: Vec<(String, String)>,
    summary: &mut OkfImportSummary,
) -> Vec<ParsedOkfDoc> {
    let mut docs = Vec::new();
    for (path, content) in files {
        match parse_okf_document(&path, &content) {
            Ok(Some(doc)) => docs.push(doc),
            Ok(None) => {}
            Err(e) => {
                summary.skipped += 1;
                summary.warnings.push(e.to_string());
            }
        }
    }
    docs
}

// ============================================
// Export: KB entries -> conformant OKF bundle
// ============================================

/// Default OKF `type` for an entry `kind` when no raw type was preserved.
fn default_type_for_kind(kind: &str) -> &'static str {
    match kind {
        "table" => "Table",
        "business" => "Business Definition",
        "query" => "Query",
        "runbook" => "Runbook",
        _ => "Note",
    }
}

/// Bundle subdirectory for an entry `kind`.
fn kind_dir(kind: &str) -> &'static str {
    match kind {
        "table" => "tables",
        "business" => "business",
        "query" => "queries",
        "runbook" => "runbooks",
        _ => "notes",
    }
}

/// Slugify a title into a filename stem.
fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for c in s.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !out.is_empty() && !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    let trimmed = out.trim_matches('-').to_string();
    if trimmed.is_empty() {
        "entry".to_string()
    } else {
        trimmed
    }
}

/// Render one entry as an OKF concept document `(path, content)`.
fn entry_to_okf_file(entry: &KnowledgeEntryRow) -> (String, String) {
    let raw_type = entry
        .tags
        .iter()
        .find_map(|t| t.strip_prefix(OKF_TYPE_TAG_PREFIX));
    let type_str = raw_type
        .map(|s| s.to_string())
        .unwrap_or_else(|| default_type_for_kind(&entry.kind).to_string());

    let path = entry
        .tags
        .iter()
        .find_map(|t| t.strip_prefix(OKF_PATH_TAG_PREFIX))
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{}/{}.md", kind_dir(&entry.kind), slugify(&entry.title)));

    // User tags only: drop reserved okf:* round-trip tags.
    let user_tags: Vec<&str> = entry
        .tags
        .iter()
        .filter(|t| !t.starts_with("okf:"))
        .map(|s| s.as_str())
        .collect();

    let mut fm = serde_yaml::Mapping::new();
    let s = |v: &str| serde_yaml::Value::String(v.to_string());
    fm.insert(s("type"), s(&type_str));
    fm.insert(s("title"), s(&entry.title));
    if let Some(resource) = &entry.resource {
        fm.insert(s("resource"), s(resource));
    }
    if !user_tags.is_empty() {
        fm.insert(
            s("tags"),
            serde_yaml::Value::Sequence(user_tags.iter().map(|t| s(t)).collect()),
        );
    }
    fm.insert(s("timestamp"), s(&entry.updated_at.to_rfc3339()));

    let fm_yaml = serde_yaml::to_string(&fm).unwrap_or_default();
    let content = format!("---\n{fm_yaml}---\n\n{}\n", entry.body.trim_end());
    (path, content)
}

/// Build a root `index.md` listing concepts grouped by kind for progressive
/// disclosure. Declares the OKF version (only index.md may carry frontmatter).
fn build_index(files: &[(String, String)]) -> String {
    let mut out =
        String::from("---\nokf_version: \"0.1\"\ntype: Index\n---\n\n# Knowledge Bundle\n\n");
    let mut paths: Vec<&String> = files.iter().map(|(p, _)| p).collect();
    paths.sort();
    for p in paths {
        out.push_str(&format!("- [{p}](/{p})\n"));
    }
    out
}

/// Build the full set of bundle files for a knowledge base's entries,
/// including the root `index.md`.
pub fn build_export_files(entries: &[KnowledgeEntryRow]) -> Vec<(String, String)> {
    let mut files: Vec<(String, String)> = entries.iter().map(entry_to_okf_file).collect();
    // Guard against duplicate paths (e.g. hand-authored entries with colliding
    // slugs) by disambiguating with a short id suffix.
    let mut seen = std::collections::HashSet::new();
    for (i, (path, _)) in files.iter_mut().enumerate() {
        if !seen.insert(path.clone()) {
            let disambiguated = match path.rsplit_once(".md") {
                Some((stem, _)) => format!("{stem}-{i}.md"),
                None => format!("{path}-{i}"),
            };
            *path = disambiguated;
            seen.insert(path.clone());
        }
    }
    let index = build_index(&files);
    files.push(("index.md".to_string(), index));
    files
}

/// Encode `(path, content)` files into a gzipped tarball.
pub fn encode_tar_gz(files: &[(String, String)]) -> Result<Vec<u8>> {
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    {
        let mut builder = tar::Builder::new(&mut encoder);
        for (path, content) in files {
            if is_unsafe_path(path) {
                anyhow::bail!("refusing to archive unsafe path: {path}");
            }
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_mtime(0);
            header.set_cksum();
            builder.append_data(&mut header, path, content.as_bytes())?;
        }
        builder.finish()?;
    }
    Ok(encoder.finish()?)
}

// ============================================
// Import command (HTTP: POST /v1/knowledge-bases/{kb_id}/okf_import)
// ============================================

/// Max files accepted in one bundle, bounding parse/import work.
const MAX_BUNDLE_FILES: usize = 10_000;
/// Max decoded tarball size (32 MiB), bounding memory for untrusted input.
const MAX_BUNDLE_BYTES: usize = 32 * 1024 * 1024;

/// A single inline bundle file, for callers that send files as JSON rather
/// than a tarball.
#[derive(Debug, Clone, serde::Deserialize, utoipa::ToSchema)]
pub struct OkfFileInput {
    /// Bundle-relative path, e.g. `tables/orders.md`.
    pub path: String,
    /// Raw markdown file content (frontmatter + body).
    pub content: String,
}

/// Request body for `okf_import`.
#[derive(Debug, Clone, serde::Deserialize, utoipa::ToSchema)]
pub struct ImportOkfBundleRequest {
    /// When true, delete previously-imported entries absent from this bundle.
    #[serde(default)]
    pub prune: bool,
    /// Inline bundle files. Mutually exclusive with `bundle_base64`.
    #[serde(default)]
    pub files: Vec<OkfFileInput>,
    /// A base64-encoded `.tar.gz` OKF bundle. Mutually exclusive with `files`.
    #[serde(default)]
    pub bundle_base64: Option<String>,
}

/// Import an OKF bundle into a knowledge base. See knowledge/runtime-resources/okf-adoption.md.
#[derive(Debug, serde::Deserialize, utoipa::ToSchema)]
pub struct ImportOkfBundle {
    /// Knowledge base's prefixed public identifier.
    pub kb_id: String,
    #[serde(default)]
    pub prune: bool,
    #[serde(default)]
    pub files: Vec<OkfFileInput>,
    #[serde(default)]
    pub bundle_base64: Option<String>,
}

impl ImportOkfBundle {
    pub fn from_request(kb_id: String, request: ImportOkfBundleRequest) -> Self {
        Self {
            kb_id,
            prune: request.prune,
            files: request.files,
            bundle_base64: request.bundle_base64,
        }
    }
}

impl crate::domains::common::Command for ImportOkfBundle {
    type Output = OkfImportSummary;

    fn meta() -> crate::domains::common::CommandMeta {
        crate::domains::common::CommandMeta {
            name: "import_okf_bundle",
            category: "knowledge_bases",
            description: "Import an Open Knowledge Format (OKF) bundle into a knowledge base.",
            method: "POST",
            path: "/v1/knowledge-bases/{kb_id}/okf_import",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&super::KNOWLEDGE_BASE_MANAGE)
    }

    fn output_schema() -> serde_json::Value {
        crate::domains::common::output_schema_for::<OkfImportSummary>()
    }

    async fn execute(
        self,
        ctx: &crate::domains::common::Ctx,
    ) -> Result<OkfImportSummary, crate::domains::common::CommandError> {
        use crate::domains::common::CommandError;
        use base64::Engine;

        let kb_internal_id =
            super::commands::resolve_kb_internal_id(ctx, &self.kb_id, true).await?;

        if self.bundle_base64.is_some() && !self.files.is_empty() {
            return Err(CommandError::bad_request(
                "Provide either `files` or `bundle_base64`, not both",
            ));
        }

        let files: Vec<(String, String)> = if let Some(b64) = self.bundle_base64 {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64.trim())
                .map_err(|_| CommandError::bad_request("bundle_base64 is not valid base64"))?;
            if bytes.len() > MAX_BUNDLE_BYTES {
                return Err(CommandError::bad_request("bundle exceeds 32 MiB limit"));
            }
            decode_tar_gz_bundle(&bytes)
                .map_err(|e| CommandError::bad_request(format!("invalid bundle: {e}")))?
        } else {
            self.files
                .into_iter()
                .map(|f| (f.path, f.content))
                .collect()
        };

        if files.len() > MAX_BUNDLE_FILES {
            return Err(CommandError::bad_request(format!(
                "bundle has too many files (max {MAX_BUNDLE_FILES})"
            )));
        }

        let mut summary = OkfImportSummary::default();
        let docs = parse_bundle(files, &mut summary);
        let run = import_parsed_bundle(&ctx.db, kb_internal_id, docs, self.prune)
            // `&ctx.db` derefs the Arc<StorageBackend> to &StorageBackend.
            .await
            .map_err(crate::domains::common::classify_anyhow)?;

        summary.created = run.created;
        summary.updated = run.updated;
        summary.pruned = run.pruned;
        summary.skipped += run.skipped;
        summary.warnings.extend(run.warnings);
        Ok(summary)
    }
}

inventory::submit! { crate::domains::common::CommandDescriptor::of::<ImportOkfBundle>() }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_maps_to_kind() {
        assert_eq!(map_type_to_kind("BigQuery Table"), "table");
        assert_eq!(map_type_to_kind("dataset"), "table");
        assert_eq!(map_type_to_kind("Weekly Active Users Metric"), "business");
        assert_eq!(map_type_to_kind("KPI Definition"), "business");
        assert_eq!(map_type_to_kind("Validated SQL"), "query");
        assert_eq!(map_type_to_kind("Incident Runbook"), "runbook");
        assert_eq!(map_type_to_kind("API Endpoint"), "note");
    }

    #[test]
    fn parses_concept_document() {
        let content = "---\n\
type: BigQuery Table\n\
title: Orders\n\
description: One row per completed customer order.\n\
resource: https://console.cloud.google.com/bigquery?t=orders\n\
tags: [Sales, revenue]\n\
---\n\
# Schema\n\
| Column | Type |\n";
        let doc = parse_okf_document("tables/orders.md", content)
            .unwrap()
            .unwrap();
        assert_eq!(doc.kind, "table");
        assert_eq!(doc.raw_type, "BigQuery Table");
        assert_eq!(doc.title, "Orders");
        assert_eq!(
            doc.resource.as_deref(),
            Some("https://console.cloud.google.com/bigquery?t=orders")
        );
        assert_eq!(doc.tags, vec!["sales".to_string(), "revenue".to_string()]);
        assert!(
            doc.body
                .starts_with("One row per completed customer order.")
        );
        assert!(doc.body.contains("# Schema"));
    }

    #[test]
    fn reserved_files_yield_no_document() {
        assert!(
            parse_okf_document("index.md", "anything")
                .unwrap()
                .is_none()
        );
        assert!(
            parse_okf_document("sales/log.md", "anything")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn missing_type_is_an_error() {
        let content = "---\ntitle: No Type\n---\nbody";
        assert!(parse_okf_document("notes/x.md", content).is_err());
    }

    #[test]
    fn missing_frontmatter_is_an_error() {
        assert!(parse_okf_document("notes/x.md", "# just a heading\n").is_err());
    }

    #[test]
    fn rejects_unsafe_paths() {
        let content = "---\ntype: note\n---\nbody";
        assert!(parse_okf_document("../escape.md", content).is_err());
        assert!(parse_okf_document("/abs.md", content).is_err());
        assert!(parse_okf_document("a/../../b.md", content).is_err());
    }

    #[test]
    fn over_length_resource_is_dropped() {
        let content = format!("---\ntype: note\nresource: {}\n---\nbody", "x".repeat(3000));
        let doc = parse_okf_document("notes/x.md", &content).unwrap().unwrap();
        assert!(doc.resource.is_none());
    }

    #[test]
    fn title_falls_back_to_filename_stem() {
        let content = "---\ntype: note\n---\nbody";
        let doc = parse_okf_document("notes/weekly_active_users.md", content)
            .unwrap()
            .unwrap();
        assert_eq!(doc.title, "weekly active users");
    }

    #[tokio::test]
    async fn import_is_idempotent_and_prunes() {
        let db = StorageBackend::in_memory();
        let kb = db
            .create_knowledge_base(
                everruns_core::DEFAULT_ORG_ID,
                crate::storage::models::CreateKnowledgeBaseRow {
                    public_id: everruns_provider::typed_id::KnowledgeBaseId::new().to_string(),
                    name: "OKF".into(),
                    description: None,
                    owner_principal_id: None,
                    resolved_owner_user_id: None,
                    embedding_model_id: None,
                },
            )
            .await
            .unwrap();

        let doc = |path: &str, resource: Option<&str>| ParsedOkfDoc {
            source_path: path.into(),
            title: "T".into(),
            body: "body".into(),
            kind: "table".into(),
            raw_type: "BigQuery Table".into(),
            resource: resource.map(|s| s.into()),
            tags: vec!["sales".into()],
        };

        // First import: two new entries.
        let s1 = import_parsed_bundle(
            &db,
            kb.id,
            vec![
                doc("tables/orders.md", Some("res://orders")),
                doc("tables/customers.md", None),
            ],
            false,
        )
        .await
        .unwrap();
        assert_eq!((s1.created, s1.updated), (2, 0));

        // Re-import same bundle: both update, none created (idempotent).
        let s2 = import_parsed_bundle(
            &db,
            kb.id,
            vec![
                doc("tables/orders.md", Some("res://orders")),
                doc("tables/customers.md", None),
            ],
            false,
        )
        .await
        .unwrap();
        assert_eq!((s2.created, s2.updated), (0, 2));

        // Import with one doc dropped + prune: orders updated, customers pruned.
        let s3 = import_parsed_bundle(
            &db,
            kb.id,
            vec![doc("tables/orders.md", Some("res://orders"))],
            true,
        )
        .await
        .unwrap();
        assert_eq!((s3.created, s3.updated, s3.pruned), (0, 1, 1));

        let remaining = db.list_knowledge_entries(kb.id, None, None).await.unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].resource.as_deref(), Some("res://orders"));
        // Reserved okf:* tags are recorded for round-trip fidelity.
        assert!(
            remaining[0]
                .tags
                .iter()
                .any(|t| t.starts_with(OKF_PATH_TAG_PREFIX))
        );
    }

    #[tokio::test]
    async fn export_round_trips_through_import() {
        let db = StorageBackend::in_memory();
        let kb = db
            .create_knowledge_base(
                everruns_core::DEFAULT_ORG_ID,
                crate::storage::models::CreateKnowledgeBaseRow {
                    public_id: everruns_provider::typed_id::KnowledgeBaseId::new().to_string(),
                    name: "OKF".into(),
                    description: None,
                    owner_principal_id: None,
                    resolved_owner_user_id: None,
                    embedding_model_id: None,
                },
            )
            .await
            .unwrap();

        let source = "---\n\
type: BigQuery Table\n\
title: Orders\n\
resource: res://orders\n\
tags: [sales]\n\
---\n\
# Schema\nstuff\n";
        let mut summary = OkfImportSummary::default();
        let docs = parse_bundle(
            vec![("tables/orders.md".into(), source.into())],
            &mut summary,
        );
        import_parsed_bundle(&db, kb.id, docs, false).await.unwrap();

        let entries = db.list_knowledge_entries(kb.id, None, None).await.unwrap();
        let files = build_export_files(&entries);

        // index.md is present and is not itself a concept document.
        let index = files.iter().find(|(p, _)| p == "index.md").unwrap();
        assert!(index.1.contains("okf_version"));
        assert!(parse_okf_document("index.md", &index.1).unwrap().is_none());

        // The orders concept re-parses to the original values (path, type, etc.).
        let (path, content) = files.iter().find(|(p, _)| p == "tables/orders.md").unwrap();
        let reparsed = parse_okf_document(path, content).unwrap().unwrap();
        assert_eq!(reparsed.raw_type, "BigQuery Table");
        assert_eq!(reparsed.kind, "table");
        assert_eq!(reparsed.title, "Orders");
        assert_eq!(reparsed.resource.as_deref(), Some("res://orders"));
        assert_eq!(reparsed.tags, vec!["sales".to_string()]);

        // The bundle encodes to a tarball that decodes back to the same files.
        let bytes = encode_tar_gz(&files).unwrap();
        let decoded = decode_tar_gz_bundle(&bytes).unwrap();
        assert!(decoded.iter().any(|(p, _)| p == "tables/orders.md"));
        assert!(decoded.iter().any(|(p, _)| p == "index.md"));
    }

    #[test]
    fn export_rejects_unsafe_archive_paths() {
        for path in ["../escape.md", "/absolute.md", "notes/../../escape.md"] {
            let files = vec![(path.to_string(), "content".to_string())];
            assert!(
                encode_tar_gz(&files).is_err(),
                "expected unsafe archive path to be rejected: {path}"
            );
        }
    }
}
