//! `agentd push ./agents`: push every agent file in a folder to an install
//! (`PUT /v1/agents/{id}`), the way a controller keeps its agents in git.
//!
//! Decisions:
//! - One file per agent; its name without the extension is the agent's id.
//!   JSON and YAML both read as the same agent definition.
//! - Unchanged agents keep their revision (the install compares), so
//!   pushing the same folder twice changes nothing.
//! - `--prune` deletes the install's agents that have no file; without it
//!   nothing is ever deleted. Sessions already running keep their copy.

use std::path::Path;

use serde_json::Value;

/// What a push did to one agent.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Pushed {
    pub id: String,
    /// `created`, `updated`, `unchanged`, `deleted`.
    pub outcome: &'static str,
    pub revision: u64,
}

/// The agent definitions in `dir`: (id, definition), by id.
pub fn read_dir(dir: &Path) -> anyhow::Result<Vec<(String, Value)>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        let text = match ext {
            "json" | "yaml" | "yml" => std::fs::read_to_string(&path)?,
            _ => continue,
        };
        let definition: Value = if ext == "json" {
            serde_json::from_str(&text)
        } else {
            serde_yaml::from_str(&text).map_err(|e| serde_json::Error::io(std::io::Error::other(e)))
        }
        .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_owned();
        anyhow::ensure!(
            crate::install::valid_name(&id),
            "{}: an agent id is letters, digits, `-`, `_`, `.`",
            path.display()
        );
        out.push((id, definition));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Push `dir` to the install at `url`.
pub async fn push(
    url: &str,
    api_key: Option<&str>,
    dir: &Path,
    prune: bool,
) -> anyhow::Result<Vec<Pushed>> {
    let agents = read_dir(dir)?;
    let http = reqwest::Client::new();
    let base = url.trim_end_matches('/');
    let auth = |request: reqwest::RequestBuilder| match api_key {
        Some(key) => request.bearer_auth(key),
        None => request,
    };
    let mut out = Vec::new();
    for (id, definition) in &agents {
        let response = auth(http.put(format!("{base}/v1/agents/{id}")))
            .json(definition)
            .send()
            .await?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or_default();
        anyhow::ensure!(status.is_success(), "agent {id}: {status} {body}");
        let revision = body["revision"].as_u64().unwrap_or(0);
        let outcome = match (body["changed"].as_bool(), revision) {
            (Some(false), _) => "unchanged",
            (_, 1) => "created",
            _ => "updated",
        };
        out.push(Pushed {
            id: id.clone(),
            outcome,
            revision,
        });
    }
    if prune {
        let listed: Value = auth(http.get(format!("{base}/v1/agents")))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;
        for entry in listed["data"].as_array().into_iter().flatten() {
            let id = entry["id"].as_str().unwrap_or_default();
            if id.is_empty() || agents.iter().any(|(a, _)| a == id) {
                continue;
            }
            auth(http.delete(format!("{base}/v1/agents/{id}")))
                .send()
                .await?
                .error_for_status()?;
            out.push(Pushed {
                id: id.to_owned(),
                outcome: "deleted",
                revision: entry["revision"].as_u64().unwrap_or(0),
            });
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_of_json_and_yaml_reads_as_agents_by_file_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("helper.json"),
            r#"{"name":"helper","instructions":"Help.","model":{"provider":"openai","model":"gpt-6.2-sol"}}"#,
        )
        .unwrap();
        std::fs::write(
            dir.path().join("triage.yaml"),
            "name: triage\ninstructions: Sort it.\nmodel:\n  provider: openai\n  model: gpt-6-luna\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("README.md"), "not an agent").unwrap();
        let agents = read_dir(dir.path()).unwrap();
        let ids: Vec<&str> = agents.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["helper", "triage"]);
        assert_eq!(agents[1].1["model"]["model"], "gpt-6-luna");
    }
}
