//! Credential-only E2B operations for the orphaned managed sandbox scan.
//!
//! E2B has no stop distinct from pause: "stop" pauses (keeping the
//! filesystem, freeing compute) and "delete" kills.

use std::collections::BTreeMap;

use everruns_contracts::session_sandbox::{OwnedSandbox, SandboxAdminError};
use serde::Deserialize;
use serde_json::Value;

use super::client::E2BClient;

/// Listing stops after this many pages; an account with more matching
/// sandboxes is finished on the next run.
const MAX_PAGES: usize = 20;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListedSandbox {
    #[serde(alias = "sandboxID")]
    sandbox_id: String,
    #[serde(default)]
    state: String,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    metadata: BTreeMap<String, Value>,
}

fn is_not_found(error: &str) -> bool {
    error.contains("404 Not Found") || error.contains("(404)")
}

fn owned(listed: ListedSandbox) -> OwnedSandbox {
    OwnedSandbox {
        external_id: listed.sandbox_id,
        labels: listed
            .metadata
            .into_iter()
            .filter_map(|(key, value)| value.as_str().map(|v| (key, v.to_string())))
            .collect(),
        active: listed.state == "running",
        state: listed.state,
        created_at: listed
            .started_at
            .as_deref()
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
            .map(|at| at.with_timezone(&chrono::Utc)),
    }
}

/// Running and paused sandboxes whose metadata carries every pair in `labels`.
pub async fn list_owned(
    client: &E2BClient,
    labels: &BTreeMap<String, String>,
) -> Result<Vec<OwnedSandbox>, SandboxAdminError> {
    let mut sandboxes = Vec::new();
    let mut next: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let (items, token) = client
            .list_sandboxes(labels, &["running", "paused"], next.as_deref())
            .await
            .map_err(SandboxAdminError::Provider)?;
        for item in items {
            let listed: ListedSandbox = serde_json::from_value(item)
                .map_err(|e| SandboxAdminError::Provider(format!("Invalid E2B sandbox: {e}")))?;
            sandboxes.push(owned(listed));
        }
        match token {
            Some(token) => next = Some(token),
            None => break,
        }
    }
    Ok(sandboxes)
}

pub async fn stop(client: &E2BClient, external_id: &str) -> Result<(), SandboxAdminError> {
    match client.pause_sandbox(external_id).await {
        Err(err) if !is_not_found(&err) => Err(SandboxAdminError::Provider(err)),
        _ => Ok(()),
    }
}

pub async fn delete(client: &E2BClient, external_id: &str) -> Result<(), SandboxAdminError> {
    match client.delete_sandbox(external_id).await {
        Err(err) if !is_not_found(&err) => Err(SandboxAdminError::Provider(err)),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{header, method, path, query_param, query_param_is_missing};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(server: &MockServer) -> E2BClient {
        E2BClient::with_base_url("key".to_string(), server.uri())
    }

    fn filter() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("everruns".to_string(), "true".to_string()),
            ("everruns.managed".to_string(), "true".to_string()),
            ("everruns.org_id".to_string(), "org_1".to_string()),
        ])
    }

    #[tokio::test]
    async fn lists_by_metadata_across_pages() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/sandboxes"))
            .and(header("x-api-key", "key"))
            .and(query_param(
                "metadata",
                "everruns=true&everruns.managed=true&everruns.org_id=org_1",
            ))
            .and(query_param("state", "running,paused"))
            .and(query_param_is_missing("nextToken"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("x-next-token", "page-2")
                    .set_body_json(json!([{
                        "sandboxID": "sb-1", "templateID": "base", "state": "running",
                        "startedAt": "2026-10-01T10:00:00Z",
                        "metadata": {"everruns": "true", "everruns.managed": "true"}
                    }])),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v2/sandboxes"))
            .and(query_param("nextToken", "page-2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!([{
                "sandboxID": "sb-2", "state": "paused", "metadata": {}
            }])))
            .expect(1)
            .mount(&server)
            .await;

        let listed = list_owned(&client(&server), &filter()).await.unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].external_id, "sb-1");
        assert!(listed[0].active);
        assert_eq!(listed[0].labels["everruns.managed"], "true");
        assert!(listed[0].created_at.is_some());
        assert_eq!(listed[1].state, "paused");
        assert!(!listed[1].active);
    }

    #[tokio::test]
    async fn list_errors_surface() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v2/sandboxes"))
            .respond_with(ResponseTemplate::new(401).set_body_string("bad key"))
            .mount(&server)
            .await;
        assert!(matches!(
            list_owned(&client(&server), &filter()).await,
            Err(SandboxAdminError::Provider(message)) if message.contains("401")
        ));
    }

    #[tokio::test]
    async fn stop_pauses_and_delete_kills_tolerating_missing() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sandboxes/sb-1/pause"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/sandboxes/sb-gone"))
            .respond_with(ResponseTemplate::new(404).set_body_string("missing"))
            .expect(1)
            .mount(&server)
            .await;
        let client = client(&server);
        stop(&client, "sb-1").await.unwrap();
        delete(&client, "sb-gone").await.unwrap();
    }
}
