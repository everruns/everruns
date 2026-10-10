//! Portable agent definitions loaded through the same codec as the Platform.
use crate::{Agent, AgentBuilder, Engine, Harness, Session};
#[cfg(feature = "agent-package-fs")]
use std::path::Path;

pub use everruns_core::agent_package::{
    Change as PackageChange, Diagnostic as PackageDiagnostic, Format as PackageFormat,
    Manifest as AgentManifest, PackageError,
};

/// An ID-free manifest and materialized assets. Provider credentials, custom
/// tool implementations and channel transports are supplied by the host.
#[derive(Clone)]
pub struct AgentPackage(
    pub(crate) everruns_core::agent_package::AgentPackage,
    pub(crate) Option<everruns_core::ScopedMcpServers>,
);
impl std::fmt::Debug for AgentPackage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentPackage")
            .field("name", &self.manifest().name)
            .field("assets", &self.manifest().initial_files.len())
            .field("mcp_bindings_resolved", &self.1.is_some())
            .finish()
    }
}
impl AgentPackage {
    /// Load an agent file, directory or ZIP from disk.
    #[cfg(feature = "agent-package-fs")]
    pub fn load(path: impl AsRef<Path>) -> Result<Self, PackageError> {
        everruns_core::agent_package::AgentPackage::load(path).map(|p| Self(p, None))
    }
    /// Parse a self-contained text definition, including legacy Markdown.
    pub fn parse(content: &str, format: PackageFormat) -> Result<Self, PackageError> {
        let p = everruns_core::agent_package::AgentPackage::parse(content, format)?;
        p.files()?;
        Ok(Self(p, None))
    }
    /// Load a self-contained archive, for example from `include_bytes!`.
    pub fn from_zip(bytes: &[u8]) -> Result<Self, PackageError> {
        everruns_core::agent_package::AgentPackage::from_zip(bytes).map(|p| Self(p, None))
    }
    /// Validate a fully materialized manifest.
    pub fn new(manifest: AgentManifest) -> Result<Self, PackageError> {
        let p = everruns_core::agent_package::AgentPackage::new(manifest)?;
        p.files()?;
        Ok(Self(p, None))
    }
    /// Authored metadata, including channel descriptions.
    pub fn manifest(&self) -> &AgentManifest {
        &self.0.manifest
    }
    /// Apply the definition to a builder with explicitly bound custom tools and
    /// capabilities. Unknown runtime dependencies fail rather than disappear.
    pub fn apply_to(&self, builder: AgentBuilder) -> Result<AgentBuilder, PackageError> {
        builder.package(self)
    }
    /// Start a builder. Bind a provider or override the model before building.
    pub fn builder(&self) -> Result<AgentBuilder, PackageError> {
        self.apply_to(Agent::builder())
    }
    /// Bind MCP placeholders explicitly. The library never reads host secrets.
    pub fn bind_mcp(
        mut self,
        resolve: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, PackageError> {
        self.1 = Some(self.0.bind_mcp(resolve)?);
        Ok(self)
    }
    /// Export as Markdown, TOML, YAML or JSON, without runtime identities.
    pub fn to_string(&self, format: PackageFormat) -> Result<String, PackageError> {
        self.0.to_string(format)
    }
    /// Export the complete package as ZIP.
    pub fn to_zip(&self) -> Result<Vec<u8>, PackageError> {
        self.0.to_zip()
    }
    /// Write a new or empty agent folder, preserving all assets.
    #[cfg(feature = "agent-package-fs")]
    pub fn write_folder(&self, destination: impl AsRef<Path>) -> Result<(), PackageError> {
        self.0.write_folder(destination)
    }
    /// Compare authored values and file hashes, ignoring ordering of sets.
    pub fn diff(&self, proposed: &Self) -> Result<Vec<PackageChange>, PackageError> {
        self.0.diff(&proposed.0)
    }
    /// Create a session with the manifest's standard harness. Custom harness
    /// names must instead be bound explicitly with `Session::harness`.
    pub fn create(&self, engine: &Engine, agent: Agent) -> Result<Session, PackageError> {
        let harness = match self.manifest().harness.as_deref() {
            None => None,
            Some("base") => Some(Harness::base()),
            Some("conversation") => Some(Harness::conversation()),
            Some("worker") => Some(Harness::worker()),
            // Retired level: files and bash now live on Bashkit Worker.
            Some("bashkit-worker" | "worker-base") => Some(Harness::bashkit_worker()),
            Some(name) => {
                return Err(failure(
                    "harness",
                    format!("bind custom harness {name:?} explicitly with Session::harness"),
                ));
            }
        };
        let session = engine.create(agent);
        if let Some(harness) = harness {
            session
                .bind_harness(harness)
                .map_err(|e| failure("harness", e))?;
        }
        Ok(session)
    }
}
pub(crate) fn failure(path: &str, message: impl std::fmt::Display) -> PackageError {
    PackageError(vec![PackageDiagnostic {
        path: path.into(),
        message: message.to_string(),
    }])
}
