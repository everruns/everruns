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
        let mut before = self.canonical()?;
        let mut after = proposed.canonical()?;
        omitted_portraits(&mut before, &mut after);
        compare("", Some(&before), Some(&after), &mut changes);
        Ok(changes)
    }
    pub fn canonical(&self) -> Result<Value> {
        let mut value = serde_json::to_value(&self.manifest).map_err(|e| error("manifest", e))?;
        if let Some(super::Avatar::Image(image)) = &self.manifest.avatar {
            let bytes = image.bytes()?;
            value["avatar"] = json!({
                "content_type": image.content_type,
                "sha256": Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect::<String>(),
                "bytes": bytes.len(),
            });
        }
        // Team portraits use the same bounded, non-body diff representation.
        if !self.manifest.agents.is_empty() {
            value["agents"] = Value::Array(
                self.manifest
                    .agents
                    .iter()
                    .map(|agent| AgentPackage::new(agent.clone())?.canonical())
                    .collect::<Result<Vec<_>>>()?,
            );
        }
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

// Omission preserves destination branding; false explicitly clears it. Keep
// the review aligned with what import actually applies, including team agents.
fn omitted_portraits(before: &mut Value, after: &mut Value) {
    if after.get("avatar").is_none() {
        if let Some(object) = before.as_object_mut() {
            object.remove("avatar");
        }
    } else if before.get("avatar").is_none()
        && after.get("avatar") == Some(&Value::Bool(false))
        && let Some(object) = after.as_object_mut()
    {
        object.remove("avatar");
    }
    if let (Some(before), Some(after)) = (
        before.get_mut("agents").and_then(Value::as_array_mut),
        after.get_mut("agents").and_then(Value::as_array_mut),
    ) {
        for next in after {
            if let Some(current) = before
                .iter_mut()
                .find(|current| current["name"] == next["name"])
            {
                omitted_portraits(current, next);
            }
        }
    }
}
