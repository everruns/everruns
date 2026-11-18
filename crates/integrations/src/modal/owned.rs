//! Credential-only Modal operations for the orphaned managed sandbox scan.
//!
//! Modal sandboxes cannot be paused from the control plane, and only running
//! ones are listed, so stop and delete both terminate: the first sighting
//! already frees the sandbox and the second never comes.

use std::collections::BTreeMap;

use everruns_contracts::session_sandbox::{OwnedSandbox, ProviderSandboxView, SandboxAdminError};

use super::client::ModalClient;

/// Listing stops after this many pages; the rest is seen on the next run.
const MAX_PAGES: usize = 20;

fn owned(info: super::proto::client::SandboxInfo) -> OwnedSandbox {
    let created_at = (info.created_at > 0.0)
        .then(|| chrono::DateTime::from_timestamp_millis((info.created_at * 1000.0) as i64))
        .flatten();
    OwnedSandbox {
        external_id: info.id,
        labels: info
            .tags
            .into_iter()
            .map(|tag| (tag.tag_name, tag.tag_value))
            .collect(),
        state: "running".to_string(),
        active: true,
        created_at,
    }
}

/// Running sandboxes in the account's default environment carrying every tag
/// in `labels`.
pub async fn list_owned(
    client: &ModalClient,
    labels: &BTreeMap<String, String>,
) -> Result<Vec<OwnedSandbox>, SandboxAdminError> {
    let tags: Vec<(String, String)> = labels.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let mut sandboxes = Vec::new();
    let mut before = 0.0;
    for _ in 0..MAX_PAGES {
        let page = client
            .list_sandboxes(&tags, before)
            .await
            .map_err(SandboxAdminError::Provider)?;
        let Some(oldest) = page
            .iter()
            .map(|info| info.created_at)
            .reduce(f64::min)
            .filter(|oldest| *oldest > 0.0)
        else {
            sandboxes.extend(page.into_iter().map(owned));
            break;
        };
        if before != 0.0 && oldest >= before {
            // No progress: the server ignored the cursor.
            sandboxes.extend(page.into_iter().map(owned));
            break;
        }
        before = oldest;
        sandboxes.extend(page.into_iter().map(owned));
    }
    Ok(sandboxes)
}

/// Terminate; terminating a finished sandbox succeeds.
pub async fn terminate(client: &ModalClient, external_id: &str) -> Result<(), SandboxAdminError> {
    match client.terminate(external_id).await {
        Err(err) if !err.contains("not found") => Err(SandboxAdminError::Provider(err)),
        _ => Ok(()),
    }
}

/// What Modal reports about one sandbox: running, or finished with Modal's
/// terminal status (`terminated`, `timeout`, `idle_timeout`, ...) and any
/// exception text.
pub async fn describe(
    client: &ModalClient,
    external_id: &str,
) -> Result<ProviderSandboxView, SandboxAdminError> {
    match client.sandbox_status(external_id).await {
        Ok(super::client::SandboxStatus::Running) => Ok(ProviderSandboxView {
            external_id: external_id.to_string(),
            exists: true,
            state: "running".to_string(),
            active: Some(true),
            ..ProviderSandboxView::default()
        }),
        Ok(super::client::SandboxStatus::Finished { status, exception }) => {
            Ok(ProviderSandboxView {
                external_id: external_id.to_string(),
                exists: true,
                state: format!("finished:{status}"),
                active: Some(false),
                error: exception,
                ..ProviderSandboxView::default()
            })
        }
        Err(err) if err.contains("not found") => Ok(ProviderSandboxView::missing(external_id)),
        Err(err) => Err(SandboxAdminError::Provider(err)),
    }
}
