//! Credential-only Daytona operations for the orphaned managed sandbox scan.
//!
//! Decision: these go through the generated Daytona client against the default
//! Daytona API. The scan has no Session, so it has no `provider_config` and
//! cannot honour a custom `api_base`; the server leaves accounts used with a
//! custom endpoint out of the scan rather than send their key elsewhere.

use std::collections::BTreeMap;

use crate::daytona::api::management::types::SandboxListItem;
use crate::daytona::client::DaytonaClient;
use everruns_contracts::session_sandbox::{OwnedSandbox, SandboxAdminError};

/// Daytona states that hold no compute. A sandbox in one of these needs no
/// stop, only the eventual delete.
const INACTIVE_STATES: &[&str] = &[
    "stopped",
    "stopping",
    "archived",
    "archiving",
    "error",
    "build_failed",
];
/// States of a sandbox that is gone or going; never reported.
const GONE_STATES: &[&str] = &["destroyed", "destroying"];

fn is_not_found(err: &str) -> bool {
    err.contains("404 Not Found") || err.contains("(404)") || err.contains("statusCode\":404")
}

fn owned(listed: SandboxListItem) -> Option<OwnedSandbox> {
    let state = listed
        .state
        .map(|state| state.0)
        .unwrap_or_default()
        .to_ascii_lowercase();
    if GONE_STATES.contains(&state.as_str()) {
        return None;
    }
    Some(OwnedSandbox {
        external_id: listed.id.filter(|id| !id.is_empty())?,
        labels: listed.labels.unwrap_or_default().into_iter().collect(),
        active: !INACTIVE_STATES.contains(&state.as_str()),
        state,
        created_at: listed
            .created_at
            .as_deref()
            .and_then(|at| chrono::DateTime::parse_from_rfc3339(at).ok())
            .map(|at| at.with_timezone(&chrono::Utc)),
    })
}

/// `GET /sandbox?labels=<json>`: every sandbox of the account carrying all of
/// `labels`, across every page.
pub async fn list_owned(
    client: &DaytonaClient,
    labels: &BTreeMap<String, String>,
) -> Result<Vec<OwnedSandbox>, SandboxAdminError> {
    let filter = serde_json::to_string(labels)
        .map_err(|e| SandboxAdminError::Provider(format!("Invalid label filter: {e}")))?;
    let items = client
        .list_sandboxes_by_labels(&filter)
        .await
        .map_err(SandboxAdminError::Provider)?;
    Ok(items.into_iter().filter_map(owned).collect())
}

pub async fn stop(client: &DaytonaClient, external_id: &str) -> Result<(), SandboxAdminError> {
    match client.stop_sandbox(external_id).await {
        Err(err) if !is_not_found(&err) => Err(SandboxAdminError::Provider(err)),
        _ => Ok(()),
    }
}

pub async fn delete(client: &DaytonaClient, external_id: &str) -> Result<(), SandboxAdminError> {
    match client.delete_sandbox(external_id).await {
        Err(err) if !is_not_found(&err) => Err(SandboxAdminError::Provider(err)),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn client(server: &MockServer) -> DaytonaClient {
        DaytonaClient::with_base_urls("key".to_string(), server.uri(), server.uri())
    }

    fn filter() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("everruns".to_string(), "true".to_string()),
            ("everruns.managed".to_string(), "true".to_string()),
        ])
    }

    #[tokio::test]
    async fn lists_by_label_json_and_maps_state() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sandbox"))
            .and(header("authorization", "Bearer key"))
            .and(query_param(
                "labels",
                r#"{"everruns":"true","everruns.managed":"true"}"#,
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "items": [
                    {"id": "sb-1", "state": "started", "createdAt": "2026-10-01T10:00:00.000Z",
                     "labels": {"everruns": "true", "everruns.managed": "true", "everruns.org_id": "org_1"}},
                    {"id": "sb-2", "state": "stopped", "labels": {"everruns": "true"}},
                    {"id": "sb-3", "state": "destroyed", "labels": {}}
                ],
                "nextCursor": null
            })))
            .expect(1)
            .mount(&server)
            .await;

        let listed = list_owned(&client(&server), &filter()).await.unwrap();
        assert_eq!(listed.len(), 2, "destroyed sandboxes are not reported");
        assert_eq!(listed[0].external_id, "sb-1");
        assert!(listed[0].active);
        assert_eq!(listed[0].labels["everruns.org_id"], "org_1");
        assert_eq!(
            listed[0].created_at.unwrap().to_rfc3339(),
            "2026-10-01T10:00:00+00:00"
        );
        assert_eq!(listed[1].state, "stopped");
        assert!(!listed[1].active);
        assert!(listed[1].created_at.is_none());
    }

    // The orphan scan must see every page: an account with more sandboxes than
    // one page holds would otherwise hide the rest from cleanup.
    #[tokio::test]
    async fn follows_next_cursor_across_pages_and_surfaces_errors() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/sandbox"))
            .and(query_param("cursor", "page-2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "items": [{"id": "sb-2", "state": "started", "labels": {}}],
                "nextCursor": null
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/sandbox"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "items": [{"id": "sb-1", "state": "started", "labels": {}}],
                "nextCursor": "page-2"
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/sandbox"))
            .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
            .mount(&server)
            .await;

        let client = client(&server);
        let listed = list_owned(&client, &filter()).await.unwrap();
        let ids: Vec<_> = listed.iter().map(|s| s.external_id.as_str()).collect();
        assert_eq!(ids, ["sb-1", "sb-2"]);
        assert!(matches!(
            list_owned(&client, &filter()).await,
            Err(SandboxAdminError::Provider(message)) if message.contains("401")
        ));
    }

    #[tokio::test]
    async fn stop_and_delete_treat_missing_sandbox_as_done() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/sandbox/sb-1/stop"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/sandbox/sb-gone"))
            .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/sandbox/sb-err"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;

        let client = client(&server);
        stop(&client, "sb-1").await.unwrap();
        delete(&client, "sb-gone").await.unwrap();
        assert!(delete(&client, "sb-err").await.is_err());
    }
}
