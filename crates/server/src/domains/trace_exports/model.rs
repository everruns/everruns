use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use utoipa::ToSchema;
use uuid::Uuid;

pub const MAX_DESTINATIONS: usize = 32;
pub const MAX_SELECTED: usize = 8;

/// How an agent combines its destinations with organization defaults.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RoutingMode {
    #[default]
    Inherit,
    Add,
    Replace,
    Off,
}

/// Customer destinations selected for future turns; never changes platform telemetry.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RoutingPolicy {
    #[serde(default)]
    /// Use organization defaults, add selections, replace defaults, or disable customer exports.
    #[schema(example = "inherit")]
    pub mode: RoutingMode,
    #[serde(default)]
    /// Distinct destination identifiers from this organization; at most eight selections.
    pub destination_ids: Vec<Uuid>,
}
impl RoutingPolicy {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.destination_ids.len() > MAX_SELECTED {
            return Err("Select at most 8 destinations");
        }
        if matches!(self.mode, RoutingMode::Inherit | RoutingMode::Off)
            && !self.destination_ids.is_empty()
        {
            return Err("Inherited and disabled routing cannot contain selections");
        }
        if self.destination_ids.iter().collect::<BTreeSet<_>>().len() != self.destination_ids.len()
        {
            return Err("Duplicate destination selection");
        }
        Ok(())
    }
    pub fn effective(&self, defaults: &[Uuid]) -> Vec<Uuid> {
        let mut ids = BTreeSet::new();
        if matches!(self.mode, RoutingMode::Inherit | RoutingMode::Add) {
            ids.extend(defaults.iter().copied());
        }
        if matches!(self.mode, RoutingMode::Add | RoutingMode::Replace) {
            ids.extend(self.destination_ids.iter().copied());
        }
        ids.into_iter().collect()
    }
}

/// Receiver protocol used for customer trace delivery.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TraceProvider {
    Braintrust,
    Langfuse,
    Langsmith,
    Otlp,
}

/// Non-secret receiver settings and explicit content capture consent.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DestinationConfig {
    /// Human-readable destination name, from 1 to 128 characters.
    #[schema(example = "Production traces")]
    pub name: String,
    /// Native Braintrust, Langfuse, LangSmith, or generic OTLP over HTTPS.
    #[schema(example = "otlp")]
    pub provider: TraceProvider,
    /// HTTPS receiver endpoint without embedded credentials, query parameters, or fragments.
    #[schema(example = "https://collector.example.com/v1/traces")]
    pub endpoint: String,
    /// Braintrust project identifier; required for Braintrust receivers.
    #[schema(example = "00000000-0000-0000-0000-000000000002")]
    pub project_id: Option<Uuid>,
    #[serde(default)]
    /// LangSmith project name for project-based workspaces.
    pub project_name: Option<String>,
    #[serde(default)]
    /// Registered LangSmith agent identifier, mutually exclusive with project_name.
    pub langsmith_agent_id: Option<String>,
    #[serde(default)]
    /// LangSmith agent environment: local, development, staging, or production.
    pub langsmith_agent_environment: Option<String>,
    #[serde(default)]
    /// Explicit consent to export message and tool content; defaults to metadata only.
    #[schema(example = false)]
    pub capture_content: bool,
    #[serde(default)]
    /// Explicit consent to export reasoning; also requires content capture.
    #[schema(example = false)]
    pub capture_reasoning: bool,
    #[serde(default = "enabled")]
    /// Whether customer exports may be sent to this receiver; false pauses delivery.
    #[schema(example = true)]
    pub enabled: bool,
}
fn enabled() -> bool {
    true
}
impl DestinationConfig {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.name.trim().is_empty() || self.name.len() > 128 {
            return Err("Name must be 1 to 128 characters");
        }
        if self.endpoint.len() > 2048 {
            return Err("Endpoint is too long");
        }
        let url = url::Url::parse(&self.endpoint).map_err(|_| "Invalid endpoint URL")?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "Use an HTTPS endpoint without credentials, query parameters, or fragments",
            );
        }
        if self.provider == TraceProvider::Braintrust && self.project_id.is_none() {
            return Err("Braintrust requires a project ID");
        }
        if self.provider == TraceProvider::Langsmith {
            match (
                &self.project_name,
                &self.langsmith_agent_id,
                &self.langsmith_agent_environment,
            ) {
                (Some(project), None, None) if !project.trim().is_empty() => {}
                (None, Some(agent), Some(environment))
                    if !agent.is_empty()
                        && agent.len() <= 63
                        && agent.as_bytes()[0].is_ascii_lowercase()
                        && agent
                            .as_bytes()
                            .last()
                            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                        && agent
                            .bytes()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
                        && matches!(
                            environment.as_str(),
                            "local" | "development" | "staging" | "production"
                        ) => {}
                _ => {
                    return Err(
                        "LangSmith requires either a project name or a valid agent ID and environment",
                    );
                }
            }
        } else if self.langsmith_agent_id.is_some() || self.langsmith_agent_environment.is_some() {
            return Err("LangSmith agent routing requires the LangSmith provider");
        }
        if self.project_name.as_ref().is_some_and(|p| {
            p.len() > 256 || !p.is_ascii() || p.bytes().any(|b| b < 32 || b == 127)
        }) {
            return Err("Project name must be 1 to 256 printable ASCII characters");
        }
        if matches!(
            self.provider,
            TraceProvider::Langfuse | TraceProvider::Langsmith
        ) && !url
            .path()
            .trim_end_matches('/')
            .ends_with("/otel/v1/traces")
        {
            return Err("Native OTLP providers require their full /otel/v1/traces endpoint");
        }
        if self.capture_reasoning && !self.capture_content {
            return Err("Reasoning capture requires content capture");
        }
        Ok(())
    }
}

/// Write-only receiver authentication, encrypted at rest and never returned.
#[derive(Clone, Serialize, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TraceSecrets {
    #[serde(default)]
    /// Authentication headers: Authorization, X-API-Key, or Api-Key. Values are write-only.
    pub headers: std::collections::BTreeMap<String, String>,
}
impl TraceSecrets {
    pub fn validate_for(&self, provider: &TraceProvider) -> Result<(), &'static str> {
        self.validate()?;
        let header = |name: &str| {
            self.headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v.as_str())
                .unwrap_or("")
        };
        match provider {
            TraceProvider::Braintrust if header("authorization").trim().is_empty() => {
                Err("Braintrust requires an Authorization header")
            }
            TraceProvider::Langfuse => {
                use base64::Engine;
                let credentials = header("authorization")
                    .strip_prefix("Basic ")
                    .and_then(|value| base64::engine::general_purpose::STANDARD.decode(value).ok())
                    .and_then(|v| String::from_utf8(v).ok());
                if credentials
                    .as_deref()
                    .and_then(|s| s.split_once(':'))
                    .is_some_and(|(public, secret)| !public.is_empty() && !secret.is_empty())
                {
                    Ok(())
                } else {
                    Err("Langfuse requires Basic authentication with public and secret keys")
                }
            }
            TraceProvider::Langsmith if header("x-api-key").trim().is_empty() => {
                Err("LangSmith requires an X-API-Key header")
            }
            _ => Ok(()),
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.headers.len() > 8 {
            return Err("At most 8 authentication headers are allowed");
        }
        let mut names = BTreeSet::new();
        for (k, v) in &self.headers {
            if !names.insert(k.to_ascii_lowercase()) {
                return Err("Duplicate authentication header");
            }
            // Restrict to authentication headers; never allow transport/routing overrides.
            if !matches!(
                k.to_ascii_lowercase().as_str(),
                "authorization" | "x-api-key" | "api-key"
            ) || v.len() > 4096
                || !v.is_ascii()
                || v.bytes().any(|b| b < 32 || b == 127)
            {
                return Err("Use Authorization, X-API-Key, or Api-Key with a valid header value");
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_routing_modes_and_deduplication() {
        let a = Uuid::new_v4();
        let b = Uuid::new_v4();
        for (mode, expected) in [
            (RoutingMode::Inherit, 1),
            (RoutingMode::Add, 2),
            (RoutingMode::Replace, 1),
            (RoutingMode::Off, 0),
        ] {
            let p = RoutingPolicy {
                mode,
                destination_ids: vec![b],
            };
            assert_eq!(p.effective(&[a]).len(), expected);
        }
        assert_eq!(
            RoutingPolicy {
                mode: RoutingMode::Add,
                destination_ids: vec![a]
            }
            .effective(&[a]),
            vec![a]
        );
    }
    #[test]
    fn rejects_ambiguous_and_unbounded_policies() {
        assert!(
            RoutingPolicy {
                mode: RoutingMode::Off,
                destination_ids: vec![Uuid::new_v4()]
            }
            .validate()
            .is_err()
        );
        assert!(
            RoutingPolicy {
                mode: RoutingMode::Add,
                destination_ids: vec![Uuid::new_v4(); 9]
            }
            .validate()
            .is_err()
        );
    }
    #[test]
    fn secrets_cannot_override_transport_or_inject_headers() {
        for (key, value) in [
            ("Host", "evil.example"),
            ("Authorization", "Bearer x\r\nHost: evil"),
            ("Content-Type", "text/plain"),
        ] {
            assert!(
                TraceSecrets {
                    headers: [(key.into(), value.into())].into()
                }
                .validate()
                .is_err()
            );
        }
    }
}
