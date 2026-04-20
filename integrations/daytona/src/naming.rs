use everruns_core::resource_names::{UniqueResourceName, unique_resource_name};
use serde_json::{Value, json};

use crate::client::DaytonaClient;
use crate::state::SandboxInfo;

pub(crate) async fn create_sandbox_with_unique_name(
    client: &DaytonaClient,
    requested_name: &str,
    mut body: Value,
) -> Result<(SandboxInfo, UniqueResourceName), String> {
    for attempt in 0..2 {
        let mut generated_name = unique_resource_name(requested_name);
        set_create_body_name(&mut body, &generated_name.canonical_name)?;

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

fn set_create_body_name(body: &mut Value, name: &str) -> Result<(), String> {
    match body {
        Value::Object(object) => {
            object.insert("name".to_string(), json!(name));
            Ok(())
        }
        _ => Err("Daytona sandbox create body must be a JSON object".to_string()),
    }
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
    fn rejects_non_object_create_body() {
        let mut body = json!(["invalid"]);

        let error = set_create_body_name(&mut body, "sandbox-name").unwrap_err();

        assert_eq!(error, "Daytona sandbox create body must be a JSON object");
        assert_eq!(body, json!(["invalid"]));
    }
}
