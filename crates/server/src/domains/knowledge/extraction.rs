//! Preserve source bytes; Markkit produces an independently versioned text view.
use serde_json::{Value, json};
pub const PROFILE: &str = "markkit-0.1.1/knowledge-v1";
pub struct Extracted {
    pub text: Option<String>,
    pub metadata: Value,
    pub warnings: Vec<String>,
    pub status: &'static str,
}
pub fn extract(bytes: &[u8], path: &str, mime: &str) -> Extracted {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    if matches!(
        ext.as_str(),
        "html" | "htm" | "docx" | "xlsx" | "xlsm" | "xls" | "ods"
    ) || mime == "text/html"
    {
        let markkit = markkit::Markkit::builder()
            .max_input_size(super::content::EXTRACT_LIMIT)
            .max_processing_secs(30)
            .max_archive_entries(2000)
            .max_archive_entry_size(16 * 1024 * 1024)
            .max_archive_total_size(64 * 1024 * 1024)
            .max_spreadsheet_cells(250_000)
            .build();
        return match markkit.extract_document_sync(
            bytes,
            markkit::DocumentHint {
                content_type: Some(mime),
                file_name: Some(path),
            },
        ) {
            Ok(doc) if doc.markdown.len() <= super::content::EXTRACT_LIMIT => Extracted {
                text: Some(doc.markdown),
                metadata: json!({"extraction":{"profile":PROFILE,"format":doc.format,"title":doc.title,"stats":doc.stats}}),
                warnings: doc.warnings,
                status: "ready",
            },
            Ok(_) => Extracted {
                text: None,
                metadata: json!({}),
                warnings: vec!["extracted_text_exceeds_limit".into()],
                status: "over_budget",
            },
            Err(_) => Extracted {
                text: None,
                metadata: json!({}),
                warnings: vec!["document_extraction_failed".into()],
                status: "failed",
            },
        };
    }
    if !bytes.contains(&0)
        && let Ok(text) = std::str::from_utf8(bytes)
    {
        let mut metadata = json!({});
        if ext == "pdf" || mime == "application/pdf" {
            return Extracted {
                text: None,
                metadata: json!({}),
                warnings: vec!["unsupported_extraction_original_retained".into()],
                status: "unsupported",
            };
        }
        if matches!(ext.as_str(), "md" | "markdown" | "mdx") {
            match frontmatter(text) {
                Ok(Some(value)) => metadata = json!({"frontmatter": value}),
                Ok(None) => {}
                Err(_) => {
                    return Extracted {
                        text: Some(text.into()),
                        metadata,
                        warnings: vec!["invalid_frontmatter_preserved".into()],
                        status: "partial",
                    };
                }
            }
        }
        return Extracted {
            text: Some(text.into()),
            metadata,
            warnings: vec![],
            status: "ready",
        };
    }
    Extracted {
        text: None,
        metadata: json!({}),
        warnings: vec!["unsupported_extraction_original_retained".into()],
        status: "unsupported",
    }
}
/// UTF-8-safe, paragraph-aware ranges. Postings retain offsets, never passage text.
pub fn fragments(text: &str) -> Vec<(usize, usize)> {
    let mut result = Vec::new();
    let mut start = 0;
    while start < text.len() {
        let mut end = (start + 6000).min(text.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        if end < text.len()
            && let Some(split) = text[start..end].rfind("\n\n")
            && split > 1500
        {
            end = start + split + 2;
        }
        if !text[start..end].trim().is_empty() {
            result.push((start, end - start));
        }
        start = end;
    }
    result
}

/// Read exact YAML fence lines without changing canonical Markdown bytes.
/// BOM and CRLF are common in repository and OKF exports.
pub fn frontmatter(text: &str) -> anyhow::Result<Option<Value>> {
    let text = text.trim_start_matches('\u{feff}');
    let mut lines = text.split_inclusive('\n');
    if lines.next().map(str::trim_end) != Some("---") {
        return Ok(None);
    }
    let mut yaml = String::new();
    for line in lines {
        if line.trim_end() == "---" {
            let value: Value = serde_yaml::from_str(&yaml)?;
            anyhow::ensure!(value.is_object(), "Frontmatter must be an object");
            return Ok(Some(value));
        }
        if yaml.len() + line.len() > 64 * 1024 {
            anyhow::bail!("Frontmatter exceeds metadata budget");
        }
        yaml.push_str(line);
    }
    anyhow::bail!("Unclosed frontmatter fence")
}
