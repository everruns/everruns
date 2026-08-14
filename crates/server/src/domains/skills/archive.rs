// Skill archive extraction and ingestion.
//
// Extracts ZIP archives into the skill store. Moved from services::skill
// as part of service deletion (EVE-316). Kept as free functions so callers
// don't need to construct a service.

use super::queries as q;
use super::types::CreateSkillRow;
use crate::kernel_imports::{Skill, everruns_provider::typed_id::SkillId, parse_skill_md};
use crate::storage::StorageBackend;
use crate::storage::models::CreateSkillFileRow;
use anyhow::{Result, anyhow};
use std::io::Read;

/// Max ZIP archive size (10 MB)
pub const MAX_ARCHIVE_SIZE: usize = 10 * 1024 * 1024;
/// Max individual file size within archive (1 MB)
const MAX_FILE_SIZE: usize = 1024 * 1024;
/// Max number of files in archive
const MAX_FILE_COUNT: usize = 100;
/// Max total decompressed size (10 MB)
const MAX_DECOMPRESSED_SIZE: usize = 10 * 1024 * 1024;

/// Create a skill from a ZIP archive.
///
/// Validates archive size, extracts SKILL.md, parses frontmatter, creates the
/// skill row + stored files. Returns the created skill.
pub async fn create_from_archive(
    db: &StorageBackend,
    org_id: i64,
    archive_data: Vec<u8>,
) -> Result<Skill> {
    if archive_data.len() > MAX_ARCHIVE_SIZE {
        return Err(anyhow!(
            "Archive too large: {} bytes (max {})",
            archive_data.len(),
            MAX_ARCHIVE_SIZE
        ));
    }

    let extracted = extract_zip_archive(&archive_data)?;

    let skill_md_content = extracted
        .skill_md
        .as_ref()
        .ok_or_else(|| anyhow!("Archive must contain a SKILL.md file"))?;

    let parsed = parse_skill_md(skill_md_content)
        .map_err(|errors| anyhow!("Invalid SKILL.md: {}", errors.join("; ")))?;

    // Duplicate name check
    if db.get_skill_by_name(org_id, &parsed.name).await?.is_some() {
        return Err(anyhow!("Skill with name '{}' already exists", parsed.name));
    }

    let public_id = SkillId::new().to_string();
    let mut metadata_map = parsed.metadata.clone();
    if !parsed.user_invocable {
        metadata_map.insert("user_invocable".to_string(), serde_json::Value::Bool(false));
    }
    if parsed.disable_model_invocation {
        metadata_map.insert(
            "disable_model_invocation".to_string(),
            serde_json::Value::Bool(true),
        );
    }
    let metadata = serde_json::to_value(&metadata_map)?;

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

    let row = db.create_skill(org_id, input).await?;

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
        db.create_skill_file(input).await?;
    }

    Ok(q::row_to_skill(&row))
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

        if file.is_dir() {
            continue;
        }

        let path = if let Some(ref pfx) = prefix {
            raw_name.strip_prefix(pfx).unwrap_or(&raw_name).to_string()
        } else {
            raw_name.clone()
        };

        // Security: path traversal check
        if path.contains("..") || path.starts_with('/') {
            return Err(anyhow!("Path traversal detected in archive: {}", raw_name));
        }

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

        let mut content = Vec::with_capacity(size);
        file.read_to_end(&mut content)?;

        if path == "SKILL.md" || path.ends_with("/SKILL.md") {
            let text =
                String::from_utf8(content).map_err(|_| anyhow!("SKILL.md must be valid UTF-8"))?;
            skill_md = Some(text);
            continue;
        }

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
        let data = create_test_zip(&[
            ("skill/SKILL.md", b"---\nname: x\ndescription: y\n---\n\nz"),
            ("skill/sub/../../../etc/passwd", b"bad"),
        ]);
        let result = extract_zip_archive(&data);
        if let Err(e) = &result {
            assert!(e.to_string().contains("traversal"));
        }
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
