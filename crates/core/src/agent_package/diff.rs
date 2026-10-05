use super::{AgentPackage, Result, error};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Semantic change; file bodies are represented by digests, never dumped.
#[derive(Debug, Clone, Serialize)]
pub struct Change {
    pub path: String,
    pub before: Option<Value>,
    pub after: Option<Value>,
}

impl AgentPackage {
    pub fn diff(&self, proposed: &Self) -> Result<Vec<Change>> {
        let mut changes = Vec::new();
        compare(
            "",
            Some(&self.canonical()?),
            Some(&proposed.canonical()?),
            &mut changes,
        );
        Ok(changes)
    }
    pub fn canonical(&self) -> Result<Value> {
        let mut value = serde_json::to_value(&self.manifest).map_err(|e| error("manifest", e))?;
        let mut files = serde_json::Map::new();
        for file in self.files()? {
            let bytes =
                crate::session_file::SessionFile::decode_content(&file.content, &file.encoding)
                    .map_err(|e| error("content", e))?;
            let digest = Sha256::digest(&bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>();
            files.insert(
                file.path.trim_start_matches('/').to_string(),
                json!({"sha256": digest, "bytes": bytes.len(), "is_readonly": file.is_readonly}),
            );
        }
        value["files"] = Value::Object(files);
        value["tags"] = json!(
            self.manifest
                .tags
                .iter()
                .collect::<std::collections::BTreeSet<_>>()
        );
        value["capabilities"] = json!(
            self.manifest
                .capabilities
                .iter()
                .map(|c| (c.capability_id(), c.config_value()))
                .collect::<std::collections::BTreeMap<_, _>>()
        );
        Ok(value)
    }
}

fn compare(path: &str, before: Option<&Value>, after: Option<&Value>, changes: &mut Vec<Change>) {
    if before == after {
        return;
    }
    if let (Some(Value::Object(before)), Some(Value::Object(after))) = (before, after) {
        let keys = before
            .keys()
            .chain(after.keys())
            .collect::<std::collections::BTreeSet<_>>();
        for key in keys {
            let child = format!("{path}/{}", key.replace('~', "~0").replace('/', "~1"));
            if path == "/files" {
                if before.get(key) != after.get(key) {
                    changes.push(Change {
                        path: child,
                        before: before.get(key).cloned(),
                        after: after.get(key).cloned(),
                    });
                }
            } else {
                compare(&child, before.get(key), after.get(key), changes);
            }
        }
    } else {
        changes.push(Change {
            path: path.into(),
            before: before.cloned(),
            after: after.cloned(),
        });
    }
}
