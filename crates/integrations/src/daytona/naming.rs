use everruns_contracts::runtime::resource_names::{UniqueResourceName, unique_resource_name};
use serde_json::Value;

use crate::daytona::api::management::types::CreateSandbox;
use crate::daytona::client::DaytonaClient;
use crate::daytona::state::SandboxInfo;
use crate::daytona::{AUTO_ARCHIVE_INTERVAL_MINUTES, AUTO_DELETE_INTERVAL_MINUTES};

/// Create request every Everruns sandbox starts from: our labels and the
/// stop/archive/delete intervals that bound an abandoned sandbox's cost.
pub(crate) fn create_body(
    labels: &serde_json::Map<String, Value>,
    auto_stop_minutes: u64,
) -> CreateSandbox {
    let minutes = |value: u64| Some(i64::try_from(value).unwrap_or(i64::MAX));
    CreateSandbox {
        auto_stop_interval: minutes(auto_stop_minutes),
        auto_archive_interval: minutes(AUTO_ARCHIVE_INTERVAL_MINUTES),
        auto_delete_interval: minutes(AUTO_DELETE_INTERVAL_MINUTES),
        labels: labels
            .iter()
            .map(|(key, value)| {
                let value = value
                    .as_str()
                    .map_or_else(|| value.to_string(), str::to_string);
                (key.clone(), value)
            })
            .collect(),
        ..CreateSandbox::default()
    }
}

pub(crate) async fn create_sandbox_with_unique_name(
    client: &DaytonaClient,
    requested_name: &str,
    mut body: CreateSandbox,
) -> Result<(SandboxInfo, UniqueResourceName), String> {
    for attempt in 0..2 {
        let mut generated_name = unique_resource_name(requested_name);
        body.name = Some(generated_name.canonical_name.clone());

        match client.create_sandbox(body.clone()).await {
            Ok(info) => {
                if let Some(actual_name) = info.name.clone() {
                    generated_name.canonical_name = actual_name;
                }
                return Ok((info, generated_name));
            }
            Err(error) if attempt == 0 && is_daytona_name_conflict(&error) => continue,
            Err(error) => return Err(error),
        }
    }

    unreachable!("loop returns success or error")
}

fn is_daytona_name_conflict(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("409")
        || lower.contains("conflict")
        || lower.contains("already exists")
        || lower.contains("already taken")
        || lower.contains("duplicate")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_conflict_errors() {
        assert!(is_daytona_name_conflict(
            "Daytona API error (409 Conflict): sandbox already exists"
        ));
        assert!(is_daytona_name_conflict(
            "Daytona API error (422 Unprocessable Entity): duplicate sandbox name"
        ));
        assert!(!is_daytona_name_conflict(
            "Daytona API error (401 Unauthorized): invalid credentials"
        ));
    }

    #[test]
    fn create_body_carries_labels_and_lifecycle_intervals() {
        let labels = serde_json::json!({"everruns": "true", "everruns.count": 3});
        let body = create_body(labels.as_object().unwrap(), 7);

        assert_eq!(body.auto_stop_interval, Some(7));
        assert_eq!(
            body.auto_archive_interval,
            Some(AUTO_ARCHIVE_INTERVAL_MINUTES as i64)
        );
        assert_eq!(
            body.auto_delete_interval,
            Some(AUTO_DELETE_INTERVAL_MINUTES as i64)
        );
        assert_eq!(body.labels["everruns"], "true");
        assert_eq!(body.labels["everruns.count"], "3");
        assert!(body.name.is_none());
    }
}
