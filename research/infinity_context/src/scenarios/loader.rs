//! Load scenarios from YAML/JSON files

use crate::types::Scenario;
use anyhow::Result;
use std::path::Path;

/// Load scenarios from a directory
pub async fn load_from_directory(path: &Path) -> Result<Vec<Scenario>> {
    let mut scenarios = Vec::new();

    if !path.exists() {
        tracing::warn!("Scenarios directory does not exist: {:?}", path);
        return Ok(scenarios);
    }

    let entries = std::fs::read_dir(path)?;

    for entry in entries {
        let entry = entry?;
        let file_path = entry.path();

        if file_path.is_file() {
            let extension = file_path.extension().and_then(|e| e.to_str());

            match extension {
                Some("yaml") | Some("yml") => {
                    tracing::debug!("Loading YAML scenario: {:?}", file_path);
                    let content = std::fs::read_to_string(&file_path)?;
                    match serde_yaml::from_str::<Scenario>(&content) {
                        Ok(scenario) => scenarios.push(scenario),
                        Err(e) => {
                            tracing::warn!("Failed to parse {:?}: {}", file_path, e);
                        }
                    }
                }
                Some("json") => {
                    tracing::debug!("Loading JSON scenario: {:?}", file_path);
                    let content = std::fs::read_to_string(&file_path)?;
                    match serde_json::from_str::<Scenario>(&content) {
                        Ok(scenario) => scenarios.push(scenario),
                        Err(e) => {
                            tracing::warn!("Failed to parse {:?}: {}", file_path, e);
                        }
                    }
                }
                _ => {
                    tracing::trace!("Skipping non-scenario file: {:?}", file_path);
                }
            }
        }
    }

    tracing::info!("Loaded {} scenarios from {:?}", scenarios.len(), path);
    Ok(scenarios)
}
