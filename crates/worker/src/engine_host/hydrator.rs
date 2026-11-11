//! Platform agent definitions hydrated on a worker (data plane v2 step 4).
//!
//! Decisions:
//! - A declaration names its model by provider and model name and its
//!   capabilities by name. The worker serves both: the model through the
//!   organization's providers (the same store a worker turn resolves
//!   credentials from), and every capability the Framework does not
//!   implement from the platform's own registry, registered for this agent
//!   only, so one agent's platform tools never collide with another's.
//! - Platform sessions write their files, as they do on the worker's host:
//!   the workspace policy is read-write (hidden and sensitive paths stay
//!   closed, which the worker's host does not enforce).
//! - The platform serves MCP (`HostMcp`): which servers a turn has depends
//!   on the session and on who sent the input, and their credentials stay
//!   on the platform. The declaration's servers, which carry `${BINDING}`
//!   placeholders, are dropped from the Engine agent, so it never connects
//!   to a server itself.
//! - The control plane copied the harness's, agent's and session's starter
//!   files into the session when it created it, as for every platform
//!   session. The declaration's files are dropped from the Engine agent:
//!   an Engine seeds an agent's files at each activation, which would put
//!   back files the agent since changed, and the platform's file store
//!   cannot seed a read-only file through it at all (prod's first Engine
//!   session never started a turn: every activation failed on a read-only
//!   handbook file and the lease went round again).
//! - Hydration is pure: the organization of an agent key is learned when its
//!   session is offered ([`PlatformHydrator::bind_key`]), before the Engine
//!   hydrates, and a key belongs to one organization for its whole life.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use everruns::Agent;
use everruns::hosting::services::{HostProviders, HostServices, is_framework_capability};
use everruns::hosting::{AgentDefinition, AgentHydrator, Hydrated};
use everruns_contracts::error::{AgentLoopError, Result};

use crate::core::capabilities::CapabilityRegistry;
use crate::worker_adapters::{OrgAdapter, WorkerAdapters};

/// Hydrates platform declarations with the platform's providers and
/// capabilities. One per worker; clones share it.
pub struct PlatformHydrator<A: WorkerAdapters> {
    adapters: A,
    capabilities: Arc<CapabilityRegistry>,
    orgs: Arc<Mutex<HashMap<String, i64>>>,
}

impl<A: WorkerAdapters> Clone for PlatformHydrator<A> {
    fn clone(&self) -> Self {
        Self {
            adapters: self.adapters.clone(),
            capabilities: self.capabilities.clone(),
            orgs: self.orgs.clone(),
        }
    }
}

impl<A: WorkerAdapters> PlatformHydrator<A> {
    pub fn new(adapters: A) -> Self {
        Self {
            capabilities: Arc::new(adapters.capability_registry()),
            adapters,
            orgs: Arc::default(),
        }
    }

    /// Record that `agent_key` belongs to `org_id`, before a session of it
    /// activates.
    pub fn bind_key(&self, agent_key: &str, org_id: i64) {
        self.orgs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(agent_key.to_string(), org_id);
    }

    fn org(&self, agent_key: &str) -> Result<i64> {
        self.orgs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(agent_key)
            .copied()
            .ok_or_else(|| {
                AgentLoopError::config(format!(
                    "agent {agent_key} has no session offered on this worker"
                ))
            })
    }
}

impl<A: WorkerAdapters> AgentHydrator for PlatformHydrator<A> {
    fn hydrate(&self, definition: &AgentDefinition) -> Result<Hydrated> {
        let org = self.org(&definition.build.agent)?;
        let package = served_by_platform(definition.agent_package()?)?;
        let config = |error: everruns::PackageError| AgentLoopError::config(error.to_string());
        let mut services = HostServices::new().providers(HostProviders::shared(
            Arc::new(OrgAdapter::new(self.adapters.clone(), org)),
            self.adapters.driver_registry(),
        ));
        let mut registered = std::collections::HashSet::new();
        for reference in &package.manifest().capabilities {
            let id = reference.capability_id();
            if is_framework_capability(id) {
                continue;
            }
            if let Some(capability) = self.capabilities.get(id)
                && registered.insert(capability.id().to_string())
            {
                services = services.register_shared(capability.clone());
            }
        }
        // Platform session files are the session's own, writable as on the
        // worker's host; hidden and sensitive paths stay closed.
        let builder = Agent::builder()
            .host_services(services)
            .workspace_policy(everruns::WorkspacePolicy::read_write());
        let agent = package
            .apply_to(builder)
            .map_err(config)?
            .build()
            .map_err(|error| AgentLoopError::config(error.to_string()))?;
        Ok(Hydrated {
            agent,
            harness: package.session_harness().map_err(config)?,
        })
    }
}

/// `package` without what the platform serves itself: its MCP servers and
/// its starter files.
fn served_by_platform(package: everruns::AgentPackage) -> Result<everruns::AgentPackage> {
    let manifest = package.manifest();
    if manifest.mcp_servers.is_empty() && manifest.initial_files.is_empty() {
        return Ok(package);
    }
    let mut manifest = manifest.clone();
    manifest.mcp_servers = Default::default();
    manifest.initial_files.clear();
    everruns::AgentPackage::new(manifest).map_err(|error| AgentLoopError::config(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn package(manifest: serde_json::Value) -> everruns::AgentPackage {
        everruns::AgentPackage::parse(&manifest.to_string(), everruns::PackageFormat::Json)
            .expect("package parses")
    }

    #[test]
    fn the_engine_agent_carries_none_of_the_declared_mcp_servers() {
        let declared = package(json!({
            "schema_version": 1,
            "name": "docs-helper",
            "instructions": "Answer from the docs.",
            "mcp_servers": {
                "docs": {
                    "url": "https://docs.example/mcp",
                    "headers": {"Authorization": "${DOCS_TOKEN}"}
                }
            }
        }));
        // As declared, the placeholder has no binding on the Engine.
        assert!(declared.apply_to(everruns::Agent::builder()).is_err());

        let served = served_by_platform(declared).unwrap();
        assert!(served.manifest().mcp_servers.is_empty());
        assert_eq!(served.manifest().instructions, "Answer from the docs.");
        assert!(served.apply_to(everruns::Agent::builder()).is_ok());
    }

    #[test]
    fn the_engine_agent_seeds_none_of_the_declared_starter_files() {
        let declared = package(json!({
            "schema_version": 1,
            "name": "company-info",
            "instructions": "Answer from the handbook.",
            "files": [
                {"path": "/handbook/index.md", "content": "# Handbook", "is_readonly": true},
                {"path": "/notes.md", "content": "draft"}
            ]
        }));
        assert_eq!(declared.manifest().initial_files.len(), 2);

        let served = served_by_platform(declared).unwrap();
        assert!(served.manifest().initial_files.is_empty());
        assert_eq!(served.manifest().instructions, "Answer from the handbook.");
    }
}
