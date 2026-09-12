use std::{io::Write, path::Path};

fn evidence(kind: &str) -> Result<&'static str, String> {
    match kind {
        "metrics" => Ok(include_str!("../metrics.txt")),
        "deployments" => Ok(include_str!("../deployments.txt")),
        "logs" => Ok(include_str!("../logs.txt")),
        "runbook" => Ok(include_str!("../runbook.md")),
        _ => Err("Choose metrics, deployments, logs, or runbook.".into()),
    }
}

#[everruns::tool]
/// Inspect one category of bundled incident evidence: metrics, deployments, logs, or runbook.
pub async fn inspect_evidence(kind: String) -> Result<String, String> {
    evidence(&kind).map(str::to_owned)
}

fn append_update(path: &Path, update: &str) -> Result<String, String> {
    if update.len() > 500 || update.trim().is_empty() {
        return Err("Provide a non-empty update of at most 500 UTF-8 bytes.".into());
    }
    let mut log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    writeln!(log, "{}", update.replace(['\n', '\r'], " ")).map_err(|e| e.to_string())?;
    Ok(format!(
        "Recorded locally in incident.log (no production change): {update}"
    ))
}

#[everruns::tool]
/// Append an evidence-backed status update to the local exercise log. No production actions.
pub async fn record_incident_update(update: String) -> Result<String, String> {
    append_update(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("incident.log"),
        &update,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn persists_updates_without_overwriting_history() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("incident.log");
        append_update(&path, "Investigating\ncheckout").unwrap();
        append_update(&path, "Rollback proposed\rnot applied").unwrap();
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "Investigating checkout\nRollback proposed not applied\n"
        );
    }
    #[test]
    fn rejects_empty_and_oversized_updates_before_writing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("incident.log");
        assert!(append_update(&path, " ").is_err());
        assert!(append_update(&path, &"é".repeat(251)).is_err());
        assert!(!path.exists());
    }
    #[test]
    fn evidence_is_scoped_and_contains_the_diagnostic_contrast() {
        assert!(evidence("metrics").unwrap().contains("catalog: 0.1%"));
        assert!(evidence("deployments").unwrap().contains("2000ms -> 200ms"));
        assert!(evidence("logs").unwrap().contains("deadline=200ms"));
        assert!(evidence("../secrets").is_err());
    }
}
