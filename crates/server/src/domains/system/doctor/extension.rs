//! Checks a host adds on top of the built-in ones.
//!
//! Decision: a host registers its checks on its `HostComposition`
//! (`.extension(Arc::new(DoctorExtensions::new(..)))`) rather than through a
//! builder method, so a distribution with its own services (an identity
//! provider, a billing system, a model gateway) reports them in the same
//! report without its code living here.

use super::{CheckArea, DoctorCheck};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

/// Upper bound for one extension's run. A hung extension costs a warning,
/// never the whole report.
pub(super) const EXTENSION_TIMEOUT: Duration = Duration::from_secs(8);

/// What an extension found: its checks and the facts it learned about the
/// install's shape (shown in the report's setup line).
#[derive(Debug, Default)]
pub struct DoctorFindings {
    pub checks: Vec<DoctorCheck>,
    pub setup: Vec<String>,
}

impl DoctorFindings {
    pub fn check(&mut self, check: DoctorCheck) {
        self.checks.push(check);
    }

    pub fn fact(&mut self, fact: impl Into<String>) {
        self.setup.push(fact.into());
    }

    pub(super) fn extend(&mut self, other: DoctorFindings) {
        self.checks.extend(other.checks);
        self.setup.extend(other.setup);
    }
}

/// A group of checks a host adds to the system doctor.
///
/// Follow the built-in checks' rules: report presence, never secret values;
/// bound every network call; and report `info` for a service this install
/// does not use rather than a failure.
#[async_trait]
pub trait DoctorExtension: Send + Sync {
    /// Short name, used when the extension does not answer in time.
    fn name(&self) -> &str;

    async fn run(&self) -> DoctorFindings;
}

/// The extensions a host registers on its `HostComposition`.
#[derive(Default, Clone)]
pub struct DoctorExtensions(Vec<Arc<dyn DoctorExtension>>);

impl DoctorExtensions {
    pub fn new(extensions: Vec<Arc<dyn DoctorExtension>>) -> Self {
        Self(extensions)
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &Arc<dyn DoctorExtension>> {
        self.0.iter()
    }
}

/// Run one extension, turning a hang into a warning.
pub(super) async fn run_bounded(extension: &dyn DoctorExtension) -> DoctorFindings {
    match tokio::time::timeout(EXTENSION_TIMEOUT, extension.run()).await {
        Ok(findings) => findings,
        Err(_) => {
            let mut findings = DoctorFindings::default();
            findings.check(
                DoctorCheck::new(
                    CheckArea::Integrations,
                    &format!("integrations.{}", extension.name()),
                    extension.name(),
                )
                .warning(format!(
                    "These checks did not finish within {} seconds.",
                    EXTENSION_TIMEOUT.as_secs()
                ))
                .why("A service they probe is slow or unreachable, so its state is unknown.")
                .fix("Check again; if it keeps timing out, look at that service's own health."),
            );
            findings
        }
    }
}
