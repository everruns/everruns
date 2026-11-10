//! The Engine's one agent declaration (data plane v2 step 4).
//!
//! Every agent an Engine hydrates from data is a [`RuntimeAgent`]: what an
//! [`AgentDefinition`](super::AgentDefinition) revision stores and the only
//! thing an executor reads. Front ends compile to it: a portable
//! [`AgentPackage`] ([`RuntimeAgent::from_package`]), an [`Agent`] built in
//! code ([`RuntimeAgent::from_agent`]), and a platform's stored agents
//! (compiled by that platform's control plane).
//!
//! Decisions:
//! - It is internal, not the public package format. Its agent layer has the
//!   portable manifest's shape today, so packages compile without loss, but
//!   it grows on its own (behind [`RUNTIME_AGENT_FORMAT`]) when the Engine
//!   needs something the public format should not carry.
//! - It names things and never points at a stored record. A layer that
//!   carries a stored agent, model or harness id is refused, so a host's
//!   ids stay in the host: the host maps a session to its own records.
//! - It is versioned. An executor reads the formats in
//!   [`RUNTIME_AGENT_FORMATS`] and refuses others before any session binds,
//!   the same rule as the session protocol, so sessions can move between
//!   executors of different versions during a deploy.
//! - Behavior that only code can carry (custom capability code, approval
//!   gates, lifecycle hooks) does not compile: such an agent stays a
//!   registered build ([`EngineBuilder::register`](crate::EngineBuilder::register)).

use crate::{Agent, AgentPackage, PackageFormat};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::ops::RangeInclusive;

use super::spi::error::{AgentLoopError, Result};

/// The declaration format this build writes.
pub const RUNTIME_AGENT_FORMAT: u32 = 1;

/// The declaration formats this build reads.
pub const RUNTIME_AGENT_FORMATS: RangeInclusive<u32> = 1..=1;

/// One agent, as the Engine runs it: id-free, versioned data. See the
/// module documentation. Stability: experimental.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RuntimeAgent {
    /// Declaration format ([`RUNTIME_AGENT_FORMAT`] when written by this
    /// build).
    pub format: u32,
    /// The agent layer: instructions, model by provider and model name,
    /// capabilities by name with their config, MCP servers with credential
    /// placeholders, tools, files and skills, already merged with whatever
    /// layers its front end resolves (a platform's harness chain and
    /// organization defaults).
    pub agent: Value,
}

impl RuntimeAgent {
    /// A declaration in the current format from an agent layer, as JSON in
    /// the portable manifest's shape. Checked by [`validate`](Self::validate)
    /// when published.
    pub fn new(agent: Value) -> Self {
        Self {
            format: RUNTIME_AGENT_FORMAT,
            agent,
        }
    }

    /// Compile a portable package.
    pub fn from_package(package: &AgentPackage) -> Result<Self> {
        refuse_stored_ids(package)?;
        let agent = serde_json::from_str(
            &package
                .to_string(PackageFormat::Json)
                .map_err(|error| AgentLoopError::config(error.to_string()))?,
        )
        .map_err(|error| AgentLoopError::config(error.to_string()))?;
        Ok(Self::new(agent))
    }

    /// Compile an agent built in code. Fails for behavior only code can
    /// carry; register such an agent instead.
    pub fn from_agent(agent: &Agent) -> Result<Self> {
        let package = agent
            .to_package()
            .map_err(|error| AgentLoopError::config(error.to_string()))?;
        Self::from_package(&package)
    }

    /// The agent layer as a package, for hydration.
    pub fn package(&self) -> Result<AgentPackage> {
        if !RUNTIME_AGENT_FORMATS.contains(&self.format) {
            return Err(AgentLoopError::config(format!(
                "runtime agent format {} is not readable by this executor (reads {}..={})",
                self.format,
                RUNTIME_AGENT_FORMATS.start(),
                RUNTIME_AGENT_FORMATS.end()
            )));
        }
        let package = AgentPackage::parse(&self.agent.to_string(), PackageFormat::Json)
            .map_err(|error| AgentLoopError::config(error.to_string()))?;
        refuse_stored_ids(&package)?;
        Ok(package)
    }

    /// Refuse a declaration this executor cannot read or that points at a
    /// stored record.
    pub fn validate(&self) -> Result<()> {
        self.package().map(drop)
    }
}

/// THREAT[TM-DURABLE-012]: a host's stored ids never enter the Engine. The
/// package codec accepts legacy ids as input only; a declaration carrying
/// one is refused rather than silently dropped, so a compiler that leaks
/// ids fails at publication.
fn refuse_stored_ids(package: &AgentPackage) -> Result<()> {
    let inner = &package.0;
    if inner.legacy_id.is_some()
        || inner.legacy_model_id.is_some()
        || inner.legacy_harness_id.is_some()
    {
        return Err(AgentLoopError::config(
            "a runtime agent names its model and harness and carries no stored ids",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "runtime_agent_tests.rs"]
mod tests;
