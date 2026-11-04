//! Credential-only boxd operations for the orphaned managed sandbox scan.
//!
//! Decision: the scan asks every region the org may use, because a machine
//! lives in one region and a listing answers for its own region only. Stop
//! hibernates (disk-only billing, nothing lost); delete destroys.

use std::collections::BTreeMap;

use everruns_contracts::session_sandbox::{OwnedSandbox, SandboxAdminError};

use super::MachineRef;
use super::client::{BoxdClient, Machine, is_not_found};

fn owned(region: &str, machine: Machine) -> OwnedSandbox {
    let active = !matches!(machine.status.as_str(), "hibernated" | "stopped");
    OwnedSandbox {
        external_id: MachineRef {
            region: region.to_string(),
            id: machine.id,
        }
        .to_string(),
        labels: machine.labels.into_iter().collect(),
        active,
        created_at: (machine.created_at > 0)
            .then(|| chrono::DateTime::from_timestamp(machine.created_at, 0))
            .flatten(),
        state: machine.status,
    }
}

/// Machines carrying every label in `labels`, in every region the org uses.
pub async fn list_owned(
    client: &BoxdClient,
    labels: &BTreeMap<String, String>,
) -> Result<Vec<OwnedSandbox>, SandboxAdminError> {
    let selector: Vec<(String, String)> =
        labels.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
    let regions = client
        .regions()
        .await
        .map_err(SandboxAdminError::Provider)?;
    let mut machines = Vec::new();
    for region in regions
        .iter()
        .filter(|r| r.org_state.is_empty() || r.org_state == "enabled")
    {
        let listed = client
            .list_machines(&region.id, &selector)
            .await
            .map_err(SandboxAdminError::Provider)?;
        machines.extend(
            listed
                .into_iter()
                // Re-checked client-side: the selector is a filter we asked
                // for, not one we can verify the server applied.
                .filter(|m| labels.iter().all(|(k, v)| m.labels.get(k) == Some(v)))
                .map(|m| owned(&region.id, m)),
        );
    }
    Ok(machines)
}

fn parse(external_id: &str) -> Result<MachineRef, SandboxAdminError> {
    MachineRef::parse(external_id).map_err(SandboxAdminError::Provider)
}

/// Hibernate; a machine already gone counts as stopped.
pub async fn hibernate(client: &BoxdClient, external_id: &str) -> Result<(), SandboxAdminError> {
    let machine = parse(external_id)?;
    match client.hibernate(&machine.region, &machine.id).await {
        Err(err) if !is_not_found(&err) => Err(SandboxAdminError::Provider(err)),
        _ => Ok(()),
    }
}

/// Destroy; destroying a machine already gone succeeds.
pub async fn destroy(client: &BoxdClient, external_id: &str) -> Result<(), SandboxAdminError> {
    let machine = parse(external_id)?;
    match client.destroy(&machine.region, &machine.id).await {
        Err(err) if !is_not_found(&err) => Err(SandboxAdminError::Provider(err)),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn owned_machines_carry_their_region_and_activity() {
        let machine = Machine {
            id: "vm_1".into(),
            status: "hibernated".into(),
            labels: [("everruns.managed".to_string(), "true".to_string())].into(),
            created_at: 1_790_000_000,
            ..Default::default()
        };
        let owned = owned("nyc", machine);
        assert_eq!(owned.external_id, "nyc/vm_1");
        assert!(!owned.active);
        assert_eq!(owned.labels["everruns.managed"], "true");
        assert!(owned.created_at.is_some());
    }
}
