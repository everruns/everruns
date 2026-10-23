//! OKF validation over canonical files. Unknown metadata and reserved navigation
//! files stay ordinary originals; no closed entry-kind conversion is involved.
use super::{types::*, *};
use serde_json::{Value, json};

impl KnowledgeService {
    /// A bounded page keeps validation useful on large Git/OKF bundles. Clients
    /// follow the catalog cursor; validation never reads a whole corpus at once.
    pub async fn validate_okf(&self, c: &Collection, q: &PageQuery) -> Result<Value, CommandError> {
        let root = path(q.path.as_deref().unwrap_or("/"))?;
        let root_index = format!("{}/index.md", root.trim_end_matches('/'));
        let page = self.files_page(c, q).await?;
        let mut issues = Vec::new();
        let mut checked = 0;
        for item in page["data"].as_array().into_iter().flatten() {
            let Some(path) = item["path"].as_str() else {
                continue;
            };
            if !path.ends_with(".md") {
                continue;
            }
            checked += 1;
            let v = &item["version"];
            let fm = v.pointer("/metadata/document/frontmatter");
            let name = path.rsplit('/').next().unwrap_or(path);
            let issue = if v["extraction_status"] != "ready" {
                Some("Markdown could not be fully validated within extraction limits")
            } else if path == root_index {
                if fm
                    .and_then(|v| v.get("okf_version"))
                    .and_then(Value::as_str)
                    != Some("0.2")
                {
                    Some("Root index must declare okf_version: \"0.2\"")
                } else if fm.and_then(Value::as_object).is_some_and(|v| v.len() != 1) {
                    Some("Root index frontmatter only declares okf_version")
                } else {
                    None
                }
            } else if name == "index.md" || name == "log.md" {
                if fm.is_some() {
                    Some("Reserved index/log files must not have concept frontmatter")
                } else {
                    None
                }
            } else if fm
                .and_then(|v| v.get("type"))
                .and_then(Value::as_str)
                .is_none_or(|v| v.trim().is_empty())
            {
                Some("Concept Markdown requires a non-empty type in YAML frontmatter")
            } else {
                None
            };
            if let Some(message) = issue {
                issues.push(json!({"path":path,"message":message}));
            }
        }
        if q.after.is_none() && self.file(c, &root_index).await.is_err() {
            issues.push(json!({"path":root_index,"message":"Bundle root index is missing"}));
        }
        Ok(
            json!({"checked":checked,"issues":issues,"next_cursor":page["next_cursor"],
            "scope":"metadata and reserved-file structure; original bytes are preserved"}),
        )
    }
}
