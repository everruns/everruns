//! MCP operating skills, embedded from the build-time allowlist in `skills/`.
//! Framework skills, contributor workflows, and organization-owned registry
//! records are outside this catalog.

use std::{collections::BTreeMap, sync::LazyLock};

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{JsonRpcResponse, MCP_PROTOCOL_VERSION_LATEST, caching};

pub(super) const EXTENSION_KEY: &str = "io.modelcontextprotocol/skills";
const FILES: &[(&str, &[u8])] = include!(concat!(env!("OUT_DIR"), "/public_skills.rs"));

struct Skill {
    entry: Value,
    files: BTreeMap<String, &'static [u8]>,
}

static CATALOG: LazyLock<Result<Vec<Skill>, String>> = LazyLock::new(build_catalog);

fn build_catalog() -> Result<Vec<Skill>, String> {
    FILES
        .iter()
        .filter(|(path, _)| path.ends_with("/SKILL.md"))
        .map(|(path, bytes)| {
            let root = path.strip_suffix("/SKILL.md").unwrap();
            let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
            // Use the shared Agent Skills validator, then parse the original
            // YAML separately so unknown/optional fields survive unchanged.
            everruns_contracts::runtime::skill::parse_skill_md(text).map_err(|e| e.join("; "))?;
            let yaml = text
                .strip_prefix("---\n")
                .and_then(|s| s.split_once("\n---").map(|(yaml, _)| yaml))
                .ok_or_else(|| format!("Missing frontmatter: {path}"))?;
            let frontmatter: Value = serde_yaml::from_str(yaml).map_err(|e| e.to_string())?;
            if frontmatter["name"].as_str() != root.rsplit('/').next() {
                return Err(format!("Skill name does not match directory: {path}"));
            }
            let prefix = format!("{root}/");
            let files: BTreeMap<_, _> = FILES
                .iter()
                .filter(|(path, _)| path.starts_with(&prefix))
                .map(|(path, bytes)| (format!("skill://{path}"), *bytes))
                .collect();
            if files.len() > 512 || files.values().map(|bytes| bytes.len()).sum::<usize>() > 16 * 1024 * 1024 {
                return Err(format!("Public skill exceeds MCP skill limits: {root}"));
            }
            let resources: Vec<_> = files
                .iter()
                .map(|(uri, bytes)| json!({
                    "uri": uri,
                    "digest": format!("sha256:{}", hex::encode(Sha256::digest(bytes))),
                    "size": bytes.len(),
                }))
                .collect();
            Ok(Skill {
                entry: json!({"uri": format!("skill://{path}"), "frontmatter": frontmatter, "resources": resources}),
                files,
            })
        })
        .collect()
}

fn response(id: Option<Value>, mut result: Value) -> JsonRpcResponse {
    caching::mark_cacheable(
        &mut result,
        MCP_PROTOCOL_VERSION_LATEST,
        caching::RESOURCES_LIST_TTL_MS,
        caching::CacheScope::Public,
    );
    JsonRpcResponse::success(id, result)
}

pub(super) fn handle_list(id: Option<Value>, params: Value) -> JsonRpcResponse {
    if !params.is_null() && !params.is_object() {
        return JsonRpcResponse::invalid_params(id, "Expected params object");
    }
    if params.get("cursor").is_some() {
        return JsonRpcResponse::invalid_params(
            id,
            "Unknown skills cursor; this catalog has one page",
        );
    }
    match &*CATALOG {
        Ok(skills) => response(
            id,
            json!({"skills": skills.iter().map(|s| &s.entry).collect::<Vec<_>>()}),
        ),
        Err(error) => JsonRpcResponse::error(id, -32603, error),
    }
}

pub(super) fn handle_get(id: Option<Value>, params: Value) -> JsonRpcResponse {
    let Some(uri) = params.get("uri").and_then(Value::as_str) else {
        return JsonRpcResponse::invalid_params(id, "Missing 'uri' in params");
    };
    match &*CATALOG {
        Ok(skills) => match skills.iter().find(|skill| skill.entry["uri"] == uri) {
            Some(skill) => response(id, json!({"skill": skill.entry})),
            None => JsonRpcResponse::invalid_params(id, format!("Unknown skill URI: {uri}")),
        },
        Err(error) => JsonRpcResponse::error(id, -32603, error),
    }
}

pub(super) fn read_resource(uri: &str) -> Result<Option<Value>, String> {
    let skills = CATALOG.as_ref().map_err(Clone::clone)?;
    let Some(bytes) = skills.iter().find_map(|skill| skill.files.get(uri)) else {
        return Ok(None);
    };
    let mime = mime_guess::from_path(uri)
        .first_or_octet_stream()
        .to_string();
    let mut content = json!({"uri": uri, "mimeType": mime});
    match std::str::from_utf8(bytes) {
        Ok(text) => content["text"] = json!(text),
        Err(_) => content["blob"] = json!(STANDARD.encode(bytes)),
    }
    Ok(Some(json!({"contents": [content]})))
}

pub(super) fn handle_directory_read(id: Option<Value>, params: Value) -> JsonRpcResponse {
    let Some(uri) = params.get("uri").and_then(Value::as_str) else {
        return JsonRpcResponse::invalid_params(id, "Missing 'uri' in params");
    };
    if !uri.starts_with("skill://") || uri.ends_with('/') {
        return JsonRpcResponse::invalid_params(id, format!("Invalid directory URI: {uri}"));
    }
    if params.get("cursor").is_some() {
        return JsonRpcResponse::invalid_params(
            id,
            "Unknown directory cursor; this catalog has one page",
        );
    }
    let skills = match &*CATALOG {
        Ok(skills) => skills,
        Err(error) => return JsonRpcResponse::error(id, -32603, error),
    };
    let mut children = BTreeMap::new();
    let prefix = format!("{uri}/");
    for skill in skills {
        for file in skill.files.keys() {
            if let Some(relative) = file.strip_prefix(&prefix) {
                let (name, mime) = match relative.split_once('/') {
                    Some((name, _)) => (name, "inode/directory".to_owned()),
                    None => (
                        relative,
                        mime_guess::from_path(file)
                            .first_or_octet_stream()
                            .to_string(),
                    ),
                };
                children.insert(
                    name.to_owned(),
                    json!({
                        "uri": format!("{prefix}{name}"), "name": name, "mimeType": mime,
                    }),
                );
            }
        }
    }
    if children.is_empty() {
        return JsonRpcResponse::invalid_params(id, format!("Unknown directory URI: {uri}"));
    }
    response(
        id,
        json!({"resources": children.into_values().collect::<Vec<_>>()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_catalog_contains_valid_complete_skill_trees() {
        let catalog = CATALOG.as_ref().expect("valid public skills");
        assert!(!catalog.is_empty());
        for skill in catalog {
            assert!(
                skill
                    .files
                    .contains_key(skill.entry["uri"].as_str().unwrap())
            );
            assert_eq!(
                skill.files.len(),
                skill.entry["resources"].as_array().unwrap().len()
            );
        }
        // Every embedded file belongs to a published manifest.
        for (path, _) in FILES {
            assert!(
                catalog
                    .iter()
                    .any(|skill| skill.files.contains_key(&format!("skill://{path}")))
            );
        }
    }
}
