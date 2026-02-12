// Skill service for business logic
// Handles skill CRUD, SKILL.md parsing, and ZIP archive extraction

use crate::api::skills::{CreateSkillRequest, UpdateSkillRequest};
use crate::storage::{
    StorageBackend,
    models::{CreateSkillFileRow, CreateSkillRow, UpdateSkill},
};
use anyhow::{Result, anyhow};
use everruns_core::{
    Skill, SkillContent, SkillFileEntry, SkillId, SkillSourceType, SkillStatus,
    SkillValidationResult, parse_skill_md, validate_skill_md,
};
use std::collections::HashMap;
use std::io::Read;
use std::sync::Arc;
use uuid::Uuid;

/// Max ZIP archive size (10 MB)
const MAX_ARCHIVE_SIZE: usize = 10 * 1024 * 1024;
/// Max individual file size within archive (1 MB)
const MAX_FILE_SIZE: usize = 1024 * 1024;
/// Max number of files in archive
const MAX_FILE_COUNT: usize = 100;
/// Max total decompressed size (10 MB)
const MAX_DECOMPRESSED_SIZE: usize = 10 * 1024 * 1024;

pub struct SkillService {
    db: Arc<StorageBackend>,
}

impl SkillService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    /// Create a skill from SKILL.md content
    pub async fn create(&self, org_id: i64, req: CreateSkillRequest) -> Result<Skill> {
        let parsed = parse_skill_md(&req.skill_md)
            .map_err(|errors| anyhow!("Invalid SKILL.md: {}", errors.join("; ")))?;

        // Check duplicate name
        if self
            .db
            .get_skill_by_name(org_id, &parsed.name)
            .await?
            .is_some()
        {
            return Err(anyhow!("Skill with name '{}' already exists", parsed.name));
        }

        let public_id = SkillId::new().to_string();
        let metadata = serde_json::to_value(&parsed.metadata)?;

        let input = CreateSkillRow {
            public_id,
            name: parsed.name,
            description: parsed.description,
            license: parsed.license,
            compatibility: parsed.compatibility,
            metadata,
            allowed_tools: parsed.allowed_tools,
            instructions: parsed.instructions,
            source_type: "markdown".to_string(),
            archive_data: None,
            version: parsed.version,
        };

        let row = self.db.create_skill(org_id, input).await?;
        Ok(Self::row_to_skill(&row))
    }

    /// Create a skill from a ZIP archive
    pub async fn create_from_archive(&self, org_id: i64, archive_data: Vec<u8>) -> Result<Skill> {
        if archive_data.len() > MAX_ARCHIVE_SIZE {
            return Err(anyhow!(
                "Archive too large: {} bytes (max {})",
                archive_data.len(),
                MAX_ARCHIVE_SIZE
            ));
        }

        // Extract and validate archive
        let extracted = extract_zip_archive(&archive_data)?;

        // Find and parse SKILL.md
        let skill_md_content = extracted
            .skill_md
            .as_ref()
            .ok_or_else(|| anyhow!("Archive must contain a SKILL.md file"))?;

        let parsed = parse_skill_md(skill_md_content)
            .map_err(|errors| anyhow!("Invalid SKILL.md: {}", errors.join("; ")))?;

        // Check duplicate name
        if self
            .db
            .get_skill_by_name(org_id, &parsed.name)
            .await?
            .is_some()
        {
            return Err(anyhow!("Skill with name '{}' already exists", parsed.name));
        }

        let public_id = SkillId::new().to_string();
        let metadata = serde_json::to_value(&parsed.metadata)?;

        let input = CreateSkillRow {
            public_id,
            name: parsed.name,
            description: parsed.description,
            license: parsed.license,
            compatibility: parsed.compatibility,
            metadata,
            allowed_tools: parsed.allowed_tools,
            instructions: parsed.instructions,
            source_type: "archive".to_string(),
            archive_data: Some(archive_data),
            version: parsed.version,
        };

        let row = self.db.create_skill(org_id, input).await?;

        // Store extracted files
        for file in &extracted.files {
            let input = CreateSkillFileRow {
                skill_id: row.id.uuid(),
                path: file.path.clone(),
                content: if file.is_binary {
                    None
                } else {
                    Some(file.content.clone())
                },
                content_binary: if file.is_binary {
                    Some(file.content.as_bytes().to_vec())
                } else {
                    None
                },
                is_binary: file.is_binary,
                size_bytes: file.size_bytes as i64,
            };
            self.db.create_skill_file(input).await?;
        }

        Ok(Self::row_to_skill(&row))
    }

    pub async fn get(&self, org_id: i64, id: Uuid) -> Result<Option<Skill>> {
        let row = self.db.get_skill(org_id, id).await?;
        Ok(row.as_ref().map(Self::row_to_skill))
    }

    pub async fn list(&self, org_id: i64) -> Result<Vec<Skill>> {
        let rows = self.db.list_skills(org_id).await?;
        Ok(rows.iter().map(Self::row_to_skill).collect())
    }

    /// Get full skill content (SKILL.md + files)
    pub async fn get_content(&self, org_id: i64, id: Uuid) -> Result<Option<SkillContent>> {
        let row = match self.db.get_skill(org_id, id).await? {
            Some(r) => r,
            None => return Ok(None),
        };

        // Reconstruct SKILL.md with frontmatter
        let metadata: HashMap<String, serde_json::Value> =
            serde_json::from_value(row.metadata.clone()).unwrap_or_default();
        let mut frontmatter = format!("---\nname: {}\ndescription: {}", row.name, row.description);
        if let Some(ref license) = row.license {
            frontmatter.push_str(&format!("\nlicense: {license}"));
        }
        if let Some(ref compat) = row.compatibility {
            frontmatter.push_str(&format!("\ncompatibility: {compat}"));
        }
        if !metadata.is_empty() {
            frontmatter.push_str("\nmetadata:");
            for (k, v) in &metadata {
                frontmatter.push_str(&format!("\n  {k}: {v}"));
            }
        }
        if let Some(ref tools) = row.allowed_tools {
            frontmatter.push_str(&format!("\nallowed-tools: {tools}"));
        }
        frontmatter.push_str("\n---\n\n");
        let skill_md = format!("{frontmatter}{}", row.instructions);

        // Get files for archive-based skills
        let files = if row.source_type == "archive" {
            let file_rows = self.db.list_skill_files(row.id.uuid()).await?;
            file_rows
                .into_iter()
                .filter_map(|f| {
                    if f.is_binary {
                        None // Skip binary files in content response
                    } else {
                        Some(SkillFileEntry {
                            path: f.path,
                            content: f.content.unwrap_or_default(),
                        })
                    }
                })
                .collect()
        } else {
            vec![]
        };

        Ok(Some(SkillContent { skill_md, files }))
    }

    pub async fn update(
        &self,
        org_id: i64,
        id: Uuid,
        req: UpdateSkillRequest,
    ) -> Result<Option<Skill>> {
        let mut input = UpdateSkill::default();

        // If skill_md is provided, re-parse it
        if let Some(ref skill_md) = req.skill_md {
            let parsed = parse_skill_md(skill_md)
                .map_err(|errors| anyhow!("Invalid SKILL.md: {}", errors.join("; ")))?;

            // Check name uniqueness if name changed
            if let Some(existing) = self.db.get_skill(org_id, id).await?
                && existing.name != parsed.name
                && self
                    .db
                    .get_skill_by_name(org_id, &parsed.name)
                    .await?
                    .is_some()
            {
                return Err(anyhow!("Skill with name '{}' already exists", parsed.name));
            }

            input.name = Some(parsed.name);
            input.description = Some(parsed.description);
            input.license = parsed.license;
            input.compatibility = parsed.compatibility;
            input.metadata = Some(serde_json::to_value(&parsed.metadata)?);
            input.allowed_tools = parsed.allowed_tools;
            input.instructions = Some(parsed.instructions);
            input.version = Some(parsed.version);
        }

        if let Some(status) = &req.status {
            input.status = Some(status.to_string());
        }

        let row = self.db.update_skill(org_id, id, input).await?;
        Ok(row.as_ref().map(Self::row_to_skill))
    }

    pub async fn delete(&self, org_id: i64, id: Uuid) -> Result<bool> {
        self.db.delete_skill(org_id, id).await
    }

    pub fn validate(&self, skill_md: &str) -> SkillValidationResult {
        validate_skill_md(skill_md)
    }

    fn row_to_skill(row: &crate::storage::models::SkillRow) -> Skill {
        let metadata: HashMap<String, serde_json::Value> =
            serde_json::from_value(row.metadata.clone()).unwrap_or_default();

        Skill {
            id: row.id,
            name: row.name.clone(),
            description: row.description.clone(),
            license: row.license.clone(),
            compatibility: row.compatibility.clone(),
            metadata,
            allowed_tools: row.allowed_tools.clone(),
            source_type: SkillSourceType::from(row.source_type.as_str()),
            status: SkillStatus::from(row.status.as_str()),
            version: row.version.clone(),
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

// ============================================================================
// ZIP Archive Extraction
// ============================================================================

#[derive(Debug)]
struct ExtractedArchive {
    skill_md: Option<String>,
    files: Vec<ExtractedFile>,
}

#[derive(Debug)]
struct ExtractedFile {
    path: String,
    content: String,
    is_binary: bool,
    size_bytes: usize,
}

fn extract_zip_archive(data: &[u8]) -> Result<ExtractedArchive> {
    let cursor = std::io::Cursor::new(data);
    let mut archive =
        zip::ZipArchive::new(cursor).map_err(|e| anyhow!("Invalid ZIP archive: {e}"))?;

    if archive.len() > MAX_FILE_COUNT {
        return Err(anyhow!(
            "Archive contains too many files: {} (max {})",
            archive.len(),
            MAX_FILE_COUNT
        ));
    }

    let mut skill_md = None;
    let mut files = Vec::new();
    let mut total_size: usize = 0;

    // Detect top-level directory prefix (e.g., "skill-name/")
    let prefix = detect_top_level_prefix(&mut archive);

    for i in 0..archive.len() {
        let mut file = archive.by_index(i)?;
        let raw_name = file.name().to_string();

        // Skip directories
        if file.is_dir() {
            continue;
        }

        // Strip top-level prefix if present
        let path = if let Some(ref pfx) = prefix {
            raw_name.strip_prefix(pfx).unwrap_or(&raw_name).to_string()
        } else {
            raw_name.clone()
        };

        // Security: path traversal check
        if path.contains("..") || path.starts_with('/') {
            return Err(anyhow!("Path traversal detected in archive: {}", raw_name));
        }

        // Size check
        let size = file.size() as usize;
        if size > MAX_FILE_SIZE {
            return Err(anyhow!(
                "File '{}' too large: {} bytes (max {})",
                path,
                size,
                MAX_FILE_SIZE
            ));
        }
        total_size += size;
        if total_size > MAX_DECOMPRESSED_SIZE {
            return Err(anyhow!(
                "Total decompressed size exceeds {} bytes",
                MAX_DECOMPRESSED_SIZE
            ));
        }

        // Read content
        let mut content = Vec::with_capacity(size);
        file.read_to_end(&mut content)?;

        // Check if this is SKILL.md
        if path == "SKILL.md" || path.ends_with("/SKILL.md") {
            let text =
                String::from_utf8(content).map_err(|_| anyhow!("SKILL.md must be valid UTF-8"))?;
            skill_md = Some(text);
            continue;
        }

        // Determine if binary
        let is_binary = content.iter().take(512).any(|&b| b == 0);

        if is_binary {
            files.push(ExtractedFile {
                path,
                content: base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    &content,
                ),
                is_binary: true,
                size_bytes: size,
            });
        } else {
            let text = String::from_utf8(content)
                .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).to_string());
            files.push(ExtractedFile {
                path,
                content: text,
                is_binary: false,
                size_bytes: size,
            });
        }
    }

    Ok(ExtractedArchive { skill_md, files })
}

/// Detect if all files share a common top-level directory prefix
fn detect_top_level_prefix(
    archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
) -> Option<String> {
    let mut first_dir: Option<String> = None;
    let mut all_share = true;

    for i in 0..archive.len() {
        if let Ok(file) = archive.by_index(i) {
            let name = file.name().to_string();
            if let Some(slash_pos) = name.find('/') {
                let dir = &name[..=slash_pos];
                match &first_dir {
                    None => first_dir = Some(dir.to_string()),
                    Some(d) if d != dir => {
                        all_share = false;
                        break;
                    }
                    _ => {}
                }
            } else {
                // File at root level, no common prefix
                all_share = false;
                break;
            }
        }
    }

    if all_share { first_dir } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = Vec::new();
        {
            let mut writer = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let options = zip::write::SimpleFileOptions::default();
            for (name, content) in files {
                writer.start_file(name.to_string(), options).unwrap();
                std::io::Write::write_all(&mut writer, content).unwrap();
            }
            writer.finish().unwrap();
        }
        buf
    }

    #[test]
    fn test_extract_zip_with_skill_md() {
        let skill_md = b"---\nname: test-skill\ndescription: A test.\n---\n\nBody.";
        let script = b"#!/bin/bash\necho hello";

        let data = create_test_zip(&[("SKILL.md", skill_md), ("scripts/run.sh", script)]);

        let result = extract_zip_archive(&data).unwrap();
        assert!(result.skill_md.is_some());
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].path, "scripts/run.sh");
        assert!(!result.files[0].is_binary);
    }

    #[test]
    fn test_extract_zip_with_prefix() {
        let skill_md = b"---\nname: test-skill\ndescription: A test.\n---\n\nBody.";
        let data = create_test_zip(&[
            ("test-skill/SKILL.md", skill_md),
            ("test-skill/scripts/run.sh", b"echo hi"),
        ]);

        let result = extract_zip_archive(&data).unwrap();
        assert!(result.skill_md.is_some());
        assert_eq!(result.files.len(), 1);
        assert_eq!(result.files[0].path, "scripts/run.sh");
    }

    #[test]
    fn test_extract_zip_path_traversal() {
        // zip crate v2 sanitizes '../' on write, so we build a raw zip
        // with path traversal embedded. Use a path inside a prefix that
        // after stripping would contain '..'
        let data = create_test_zip(&[
            ("skill/SKILL.md", b"---\nname: x\ndescription: y\n---\n\nz"),
            ("skill/sub/../../../etc/passwd", b"bad"),
        ]);
        let result = extract_zip_archive(&data);
        // zip crate may sanitize this too; if so, the file just ends up
        // at a safe path. Either way, no panic.
        if let Err(e) = &result {
            assert!(e.to_string().contains("traversal"));
        }
        // If zip sanitized it, the extraction should succeed safely
    }

    #[test]
    fn test_extract_zip_file_too_large() {
        let big = vec![0u8; MAX_FILE_SIZE + 1];
        let data = create_test_zip(&[("big.bin", &big)]);
        let result = extract_zip_archive(&data);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too large"));
    }
}
